use std::{
    cell::Cell,
    fs::{create_dir_all, File},
    io::{stdin, stdout, Read, Write},
    process::ExitCode,
    rc::Rc,
    sync::{Arc, Mutex},
    time::Instant,
};

use chrono::Local;
use clap_v3::{App, Arg, ArgMatches};

use confy;

use dirs::home_dir;
use exitcode;
use futures::executor::block_on;
use futures::{future, stream::StreamExt, task::Poll};

use log::{debug, error, info, trace, warn, SetLoggerError};
use rand::Rng;
use tokio::time;

use tibberator::tibber::tui::{self, AppState};
use tibberator::tibber::{
    check_user_shutdown, connect_live_measurement, get_home_ids, loop_for_data, AccessConfig,
    Config, LiveMeasurementSubscription, LoopEndingError,
};

use tibberator::html_logger::{HtmlLogger, LogConfig};

/// Retrieves the configuration settings for the Tibberator application.
///
/// This function reads configuration values from command-line arguments and stores them using the `confy` crate.
/// It returns a `Result<Config, confy::ConfyError>` containing the loaded configuration.
///
/// # Errors
/// Returns a `Result<Config, confy::ConfyError>`:
/// - `Ok(config)`: Successfully loaded the configuration.
/// - `Err(error)`: An error occurred during configuration loading or storage.
///
fn get_config() -> Result<Config, confy::ConfyError> {
    let matches = get_matcher();
    let app_name = "Tibberator";
    let config_name = "config";

    if let Some(access_token) = matches.value_of("token") {
        let mut config: Config = confy::load("Tibberator", "config")?;
        config.access.token = String::from(access_token);
        confy::store(app_name, config_name, config)?;
    };

    if let Some(home_id) = matches.value_of("home_id") {
        let mut config: Config = confy::load("Tibberator", "config")?;
        config.access.home_id = String::from(home_id);
        confy::store(app_name, config_name, config)?;
    }

    if let Some(display_mode) = matches.value_of("display_mode") {
        let mut config: Config = confy::load("Tibberator", "config")?;
        config.output.display_mode = match display_mode {
            "consumption" => tibberator::tibber::output::DisplayMode::Consumption,
            _ => tibberator::tibber::output::DisplayMode::Prices,
        };
        confy::store(app_name, config_name, config)?;
    }

    let config: Config = confy::load(app_name, config_name)?;
    Ok(config)
}

fn init_html_logger(file_path: &str, log_config: &LogConfig) -> Result<(), SetLoggerError> {
    let file = File::create(file_path).expect("Log file must be created.");
    HtmlLogger::init(log_config.level(), file)
}

build_info::build_info!(fn build_info);

#[tokio::main]
async fn main() -> ExitCode {
    let config = get_config().expect("Config file must be loaded.");
    if !block_on(check_home_id(&config.access)) {
        eprintln!("Home ID not found for specified access token");
        println!("Press Enter to continue...");

        let _ = stdin().read(&mut [0u8]).unwrap();
        std::process::exit(exitcode::DATAERR)
    }

    let timestamp = Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
    let file_name = format!("app_log_{}.html", timestamp);
    let file_path = home_dir()
        .expect("Home directory must be found.")
        .join(".tibberator")
        .join("logs");
    if !file_path.exists() {
        create_dir_all(&file_path).expect("Failed to create log directory");
    }
    let file_path = file_path.join(file_name);

    // Initialize the HTML logger
    init_html_logger(&file_path.to_str().unwrap(), &config.logging).unwrap();
    info!(target: "tibberator.app", "{:#?}", build_info());
    info!(target: "tibberator.app", "Application started.");

    // Initialize the TUI
    let mut terminal = tui::init_terminal().expect("Failed to initialize terminal");

    // Clone the necessary parts of config for the subscription thread
    let access_config = config.access.clone();
    let output_config = config.output.clone();
    let logging_config = config.logging.clone();

    // Create app state
    let app_state = Arc::new(Mutex::new(AppState {
        should_quit: false,
        measurement: None,
        price_info: None,
        bar_graph_data: None,
        display_mode: config.output.display_mode,
        status: String::from("Waiting for data..."),
    }));

    // Clone app_state for the subscription thread
    let app_state_clone = Arc::clone(&app_state);

    let local = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Could not create local thread.");

    // Start subscription in a separate thread
    let subscription_handle = std::thread::spawn(move || {
        local.block_on(async {
            let config_for_thread = Config {
                access: access_config,
                output: output_config,
                logging: logging_config,
            };
            let subscription_result =
                subscription_loop_tui(config_for_thread, app_state_clone).await;
            match subscription_result {
                Ok(Some(subscription)) => match subscription.stop().await {
                    Ok(_) => {
                        info!(target: "tibberator.app", "Subscription stopped successfully");
                        Ok(())
                    }
                    Err(err) => {
                        error!(target: "tibberator.app", "Failed to stop subscription: {:?}", err);
                        Err(Box::<dyn std::error::Error + Send + Sync>::from(
                            std::io::Error::new(std::io::ErrorKind::Other, err.to_string()),
                        ))
                    }
                },
                Ok(None) => {
                    info!(target: "tibberator.app", "Subscription was not established");
                    Ok(())
                }
                Err(err) => {
                    error!(target: "tibberator.app", "Failed to establish subscription: {:?}", err);
                    Err(err)
                }
            }
        })
    });

    // Main event loop
    while !app_state.lock().unwrap().should_quit {
        // Draw the UI
        terminal
            .draw(|f| tui::draw_ui(f, &app_state.lock().unwrap()))
            .unwrap();

        // Handle events
        if let Err(err) = tui::handle_events(&mut app_state.lock().unwrap()) {
            error!(target: "tibberator.app", "Error handling events: {:?}", err);
            break;
        }
    }

    // Restore terminal
    tui::restore_terminal(&mut terminal).unwrap();

    // Signal the subscription thread to stop
    {
        let mut state = app_state.lock().unwrap();
        state.should_quit = true;
    }

    // Wait for subscription thread to finish
    let subscription = subscription_handle.join();

    let exitcode = match subscription {
        Ok(result) => match result {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                error!(target: "tibberator.app", "Exiting application due to error: {:?}", error);
                ExitCode::FAILURE
            }
        },
        Err(error) => {
            error!(target: "tibberator.app", "Error in subscription thread: {:?}", error);
            ExitCode::FAILURE
        }
    };

    info!(target: "tibberator.app", "Application stopped.");
    log::logger().flush();

    exitcode
}

fn get_matcher() -> ArgMatches {
    App::new("Tibberator")
        .version("0.3.0")
        .author("Stephan Z. <https://github.com/babelfischchen>")
        .about(
            "Tibberator connects to the Tibber API and shows basic usage statistics for your home.
In order to work properly you need to configure your access_token and home_id in the
config.yaml found in the Tibberator app_data directory.",
        )
        .arg(
            Arg::with_name("token")
                .short('t')
                .long("token")
                .value_name("access_token")
                .help("Sets a custom access_token to access your Tibber data.")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("home_id")
                .short('h')
                .long("homeid")
                .value_name("home_id")
                .help("Sets a custom home_id to access your Tibber data.")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("display_mode")
                .short('d')
                .long("display")
                .value_name("mode")
                .help("Sets the display mode: 'prices' or 'consumption'")
                .takes_value(true)
                .possible_values(&["prices", "consumption"]),
        )
        .get_matches()
}

/// Checks if the given `home_id` exists in the list of home IDs obtained from the `AccessConfig`.
///
/// # Arguments
/// - `access_config`: A reference to an `AccessConfig` containing the necessary configuration.
///
/// # Returns
/// - `true` if the `home_id` exists in the list of home IDs, otherwise `false`.
///
async fn check_home_id(access_config: &AccessConfig) -> bool {
    match get_home_ids(&access_config).await {
        Ok(home_ids) => home_ids.contains(&access_config.home_id),
        Err(error) => {
            eprintln!("{:?}", error);
            println!("Press Enter to continue...");
            let _ = stdin().read(&mut [0u8]).unwrap();
            std::process::exit(exitcode::DATAERR);
        }
    }
}

/// Handles reconnection logic for a live measurement subscription.
///
/// This asynchronous function takes the following parameters:
/// - `access_config`: A reference to an `AccessConfig` struct containing access configuration details.
/// - `subscription`: A boxed `Subscription<StreamingOperation<LiveMeasurement>>` representing the existing subscription.
/// - `receiver`: A reference to a `Receiver<bool>` used for checking user shutdown signals.
///
/// The function performs the following steps:
/// 1. Stops the existing subscription using `subscription.stop().await`.
/// 2. Prints any error encountered during subscription stopping.
/// 3. Exits the process with an exit code of `exitcode::PROTOCOL` if an error occurred.
/// 4. Generates a random number of seconds (between 1 and 60) to wait before reconnecting.
/// 5. Prints a message indicating the waiting time.
/// 6. Loops for the specified number of seconds, checking for user shutdown signals.
/// 7. If a shutdown signal is received, returns an `Err(LoopEndingError::Shutdown)`.
/// 8. Otherwise, reconnects by calling `connect_live_measurement(&access_config).await`.
///
/// # Errors
/// Returns an `Err(LoopEndingError::Shutdown)` if the user initiates a shutdown during the waiting period.
/// Otherwise, returns a `Result<Subscription<StreamingOperation<LiveMeasurement>>, LoopEndingError>`.
///
async fn handle_reconnect(
    access_config: &AccessConfig,
    subscription: Box<LiveMeasurementSubscription>,
    receiver: &std::sync::mpsc::Receiver<bool>,
) -> Result<LiveMeasurementSubscription, LoopEndingError> {
    // Stop the existing subscription
    subscription.stop().await.map_err(|error| {
        eprintln!("{:?}", error);
        std::process::exit(exitcode::PROTOCOL)
    })?;

    let number_of_seconds = rand::thread_rng().gen_range(1..=60);
    println!(
        "\nConnection lost, waiting for {}s before reconnecting.",
        number_of_seconds
    );

    // Wait for the specified number of seconds, checking for shutdown
    for _ in 0..=number_of_seconds {
        if check_user_shutdown(&receiver) {
            return Err(LoopEndingError::Shutdown);
        }
        std::thread::sleep(time::Duration::from_secs(1));
    }

    Ok(connect_live_measurement(&access_config).await)
}

async fn handle_reconnect_tui(
    access_config: &AccessConfig,
    subscription: Box<LiveMeasurementSubscription>,
    app_state: Arc<Mutex<AppState>>,
) -> Result<LiveMeasurementSubscription, LoopEndingError> {
    // Stop the existing subscription
    subscription.stop().await.map_err(|error| {
        eprintln!("{:?}", error);
        std::process::exit(exitcode::PROTOCOL)
    })?;

    let number_of_seconds = rand::thread_rng().gen_range(1..=60);
    println!(
        "\nConnection lost, waiting for {}s before reconnecting.",
        number_of_seconds
    );

    // Wait for the specified number of seconds, checking for shutdown
    for _ in 0..=number_of_seconds {
        if app_state.lock().unwrap().should_quit {
            return Err(LoopEndingError::Shutdown);
        }
        std::thread::sleep(time::Duration::from_secs(1));
    }

    Ok(connect_live_measurement(&access_config).await)
}

/// Handles the main subscription loop for receiving live measurement data with TUI updates.
///
/// This asynchronous function takes the following parameters:
/// - `config`: A `Config` struct containing configuration details.
/// - `receiver`: A `Receiver<bool>` used for checking user shutdown signals.
/// - `app_state`: A shared reference to the application state for updating the UI.
///
/// # Errors
/// Returns a `Result<Option<Box<Subscription<StreamingOperation<LiveMeasurement>>>>, Box<dyn std::error::Error>>`:
/// - `Ok(Some(subscription))`: Successfully processed data, returning the updated subscription.
/// - `Ok(None)`: User initiated shutdown during reconnection.
/// - `Err(error)`: An error occurred during data processing or reconnection.
///
async fn subscription_loop_tui(
    config: Config,
    app_state: Arc<Mutex<AppState>>,
) -> Result<Option<Box<LiveMeasurementSubscription>>, Box<dyn std::error::Error + Send + Sync>> {
    // Initialize subscription
    let mut subscription = Box::new(connect_live_measurement(&config.access).await);

    // Update app state status
    {
        let mut state = app_state.lock().unwrap();
        state.status = String::from("Connected, waiting for data...");
    }

    // Fetch initial price info
    match tibberator::tibber::update_current_energy_price_info(&config.access, None).await {
        Ok(price_info) => {
            // Update app state with price info
            let mut state = app_state.lock().unwrap();
            state.price_info = Some(price_info);
        }
        Err(err) => {
            error!(target: "tibberator.mainloop", "Failed to fetch price info: {:?}", err);
        }
    }

    // Fetch display data based on display mode
    match tibberator::tibber::fetch_display_data(&config).await {
        Ok(Some((data, label))) => {
            // Update app state with bar graph data
            let mut state = app_state.lock().unwrap();
            state.bar_graph_data = Some((data, label.to_string()));
        }
        Ok(None) => {
            info!(target: "tibberator.mainloop", "No display data available");
        }
        Err(err) => {
            error!(target: "tibberator.mainloop", "Failed to fetch display data: {:?}", err);
        }
    }

    // Create a custom data handler that updates the app state
    let data_handler =
        |data: tibberator::tibber::live_measurement::LiveMeasurementLiveMeasurement| {
            // Update app state with measurement data
            let mut state = app_state.lock().unwrap();
            state.measurement = Some(data);
            state.status = String::from("Connected");
        };

    // Create a custom error handler that updates the app state
    let error_handler = |error: &str| {
        // Update app state status
        let mut state = app_state.lock().unwrap();
        error!(target: "tibberator.mainloop", "Error fetching display data: {}", error);
        state.status = format!("Error: {}", error);
    };

    // Create a custom reconnect handler that updates the app state
    let reconnect_handler = || {
        // Update app state status
        let mut state = app_state.lock().unwrap();
        warn!(target: "tibberator.mainloop", "Reconnecting...");
        state.status = String::from("Reconnecting...");
    };

    // Main subscription loop
    while !app_state.lock().unwrap().should_quit {
        // Process data from subscription
        let result = process_subscription_data(
            &config,
            app_state.clone(),
            subscription.as_mut(),
            &data_handler,
            &error_handler,
            &reconnect_handler,
        )
        .await;

        match result {
            Ok(()) => {
                // Normal operation
                trace!(target: "tibberator.mainloop", "Data processed successfully");
            }
            Err(error) => {
                match *error {
                    LoopEndingError::Reconnect | LoopEndingError::ConnectionTimeout => {
                        // Handle reconnection
                        reconnect_handler();

                        match handle_reconnect_tui(&config.access, subscription, app_state.clone())
                            .await
                        {
                            Ok(new_subscription) => {
                                subscription = Box::new(new_subscription);

                                // Update app state status
                                let mut state = app_state.lock().unwrap();
                                state.status = String::from("Reconnected, waiting for data...");
                            }
                            Err(LoopEndingError::Shutdown) => {
                                // User requested shutdown during reconnection
                                return Ok(None);
                            }
                            Err(err) => {
                                error_handler(&format!("Reconnection failed: {:?}", err));
                                return Err(Box::new(err));
                            }
                        }
                    }
                    LoopEndingError::InvalidData => {
                        error_handler("Invalid data received");
                        return Err(Box::<dyn std::error::Error + Send + Sync>::from(
                            std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid Data"),
                        ));
                    }
                    LoopEndingError::Shutdown => {
                        // User requested shutdown
                        return Ok(None);
                    }
                }
            }
        }

        // Check if we should quit
        if app_state.lock().unwrap().should_quit {
            break;
        }
    }

    Ok(Some(subscription))
}

/// Process data from a subscription and call appropriate handlers
async fn process_subscription_data<F, E, R>(
    config: &Config,
    app_state: Arc<Mutex<AppState>>,
    subscription: &mut LiveMeasurementSubscription,
    data_handler: &F,
    error_handler: &E,
    reconnect_handler: &R,
) -> Result<(), Box<LoopEndingError>>
where
    F: Fn(tibberator::tibber::live_measurement::LiveMeasurementLiveMeasurement),
    E: Fn(&str),
    R: Fn(),
{
    let last_value_received = Rc::new(Cell::new(Instant::now()));
    // Create a stop function that checks for user shutdown or reconnect timeout
    let stop_fun = future::poll_fn(|_cx| {
        if app_state.lock().unwrap().should_quit {
            Poll::Ready(LoopEndingError::Shutdown)
        } else if last_value_received.get().elapsed().as_secs() > config.access.reconnect_timeout {
            Poll::Ready(LoopEndingError::Reconnect)
        } else {
            Poll::Pending
        }
    });

    let mut stream = subscription.take_until(stop_fun);

    // Process data from the stream
    match tokio::time::timeout(
        std::time::Duration::from_secs(2 * config.access.reconnect_timeout),
        stream.by_ref().next(),
    )
    .await
    {
        Ok(Some(result)) => match result.unwrap().data {
            Some(data) => {
                let current_state = data.live_measurement.unwrap();
                last_value_received.set(Instant::now());

                debug!(target: "tibberator.mainloop", "Received power measurement: {} W", current_state.power);

                // Call the data handler
                data_handler(current_state);
            }
            None => {
                error!(target: "tibberator.mainloop", "Invalid data received, shutting down.");

                // Call the error handler
                error_handler("Invalid data received");

                return Err(Box::new(LoopEndingError::InvalidData));
            }
        },
        Ok(None) => {
            // Stream ended normally
            return Ok(());
        }
        Err(_) => {
            info!(target: "tibberator.mainloop", "Connection timed out, reconnecting...");

            // Call the reconnect handler
            reconnect_handler();

            return Err(Box::new(LoopEndingError::Reconnect));
        }
    }

    // Check the result of the stream
    match stream.take_result() {
        Some(LoopEndingError::Shutdown) => {
            info!(target: "tibberator.mainloop", "User shutdown requested.");
            Err(Box::new(LoopEndingError::Shutdown))
        }
        Some(LoopEndingError::Reconnect) | Some(LoopEndingError::ConnectionTimeout) => {
            reconnect_handler();
            Err(Box::new(LoopEndingError::Reconnect))
        }
        _ => Ok(()),
    }
}

/// Original subscription loop function, kept for compatibility with tests
async fn subscription_loop(
    config: Config,
    receiver: std::sync::mpsc::Receiver<bool>,
) -> Result<Option<Box<LiveMeasurementSubscription>>, Box<dyn std::error::Error>> {
    let mut subscription = Box::new(connect_live_measurement(&config.access).await);

    write!(stdout(), "Waiting for data ...").unwrap();
    stdout().flush().unwrap();

    loop {
        let final_result = loop_for_data(&config, subscription.as_mut(), &receiver).await;
        match final_result {
            Ok(()) => break,
            Err(error) => match error.downcast_ref::<LoopEndingError>() {
                Some(LoopEndingError::Reconnect) | Some(LoopEndingError::ConnectionTimeout) => {
                    let subscription_result =
                        handle_reconnect(&config.access, subscription, &receiver).await;

                    match subscription_result {
                        Ok(res) => subscription = Box::new(res),
                        Err(LoopEndingError::Shutdown) => {
                            return Ok(None);
                        }
                        Err(error) => {
                            eprintln!("Unexpected error: {:?}", error);
                            return Err(Box::new(error) as Box<dyn std::error::Error>);
                        }
                    }
                }
                Some(LoopEndingError::InvalidData) => {
                    return Err(Box::new(LoopEndingError::InvalidData) as Box<dyn std::error::Error>)
                }
                _ => {
                    eprintln!("Unexpected error: {:?}", error);
                    return Err(Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "Invalid data",
                    )) as Box<dyn std::error::Error>);
                }
            },
        }
    }

    Ok(Some(subscription))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{env, sync::mpsc::channel};
    use tokio::time::sleep;

    fn get_test_config() -> Config {
        let current_dir = env::current_dir().expect("Failed to get current directory.");
        let filename = "tests/test-config.toml";
        let path = current_dir.join(filename);
        confy::load_path(path).expect("Config file not found.")
    }

    #[tokio::test]
    async fn test_subscription_loop() {
        let mut config = get_test_config();
        // Ensure display_mode is set for the test
        config.output.display_mode = tibberator::tibber::output::DisplayMode::Prices;
        let (sender, receiver) = channel();
        let subscription = subscription_loop(config, receiver);
        sleep(time::Duration::from_secs(5)).await;
        sender.send(true).unwrap();
        let subscription = subscription.await;
        assert!(subscription.as_ref().is_ok());
        let result = subscription.unwrap();
        assert!(result.is_some());
        assert!(result.unwrap().stop().await.is_ok());
    }

    #[tokio::test]
    async fn test_check_home_id() {
        let mut config = get_test_config();
        assert!(check_home_id(&config.access).await);
        config.access.home_id.pop();
        assert!(!check_home_id(&config.access).await);
    }
}
