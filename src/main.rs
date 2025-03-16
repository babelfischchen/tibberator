use std::{
    cell::Cell,
    collections::HashMap,
    fs::{create_dir_all, File},
    io::{stdin, stdout, Read, Write},
    ops::DerefMut,
    process::ExitCode,
    rc::Rc,
    sync::{Arc, Mutex},
    thread::sleep,
    time::Instant,
};

use chrono::Local;
use clap_v3::{App, Arg, ArgMatches};

use confy;

use dirs::home_dir;
use exitcode;
use futures::{executor::block_on, future, stream::StreamExt, task::Poll};

use log::{debug, error, info, trace, warn, SetLoggerError};
use rand::Rng;
use tokio::time;

use tibberator::{
    html_logger::{HtmlLogger, LogConfig},
    tibber::{
        cache_expired, connect_live_measurement, estimate_daily_fees, get_home_ids, loop_for_data,
        output::{self, GuiMode},
        tui::{self, AppState},
        AccessConfig, Config, LiveMeasurementSubscription, LoopEndingError,
    },
};

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
        estimated_daily_fees: None,
        cached_bar_graph: HashMap::new(),
        display_mode: config.output.display_mode,
        status: String::from("Waiting for data..."),
        data_needs_refresh: false,
    }));

    // Clone app_state for the subscription thread
    let app_state_clone = Arc::clone(&app_state);

    let local = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Could not create local thread.");

    // Start subscription in a separate thread
    let subscription_handle =
        std::thread::spawn(move || {
            local.block_on(async {
            let config_for_thread = Config {
                access: access_config,
                output: output_config,
                logging: logging_config,
            };
            let subscription_result = match &config_for_thread.output.gui_mode {
                GuiMode::Simple => subscription_loop(config_for_thread, app_state_clone).await,
                GuiMode::Advanced => subscription_loop_tui(config_for_thread, app_state_clone).await
            };
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
        if config.output.gui_mode == output::GuiMode::Advanced {
            // Draw the UI
            terminal
                .draw(|f| tui::draw_ui(f, &app_state.lock().unwrap()))
                .unwrap();
        } else {
            sleep(time::Duration::from_secs(1));
        }

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
        .version("0.5.0")
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

/// Handles the reconnection process for the TUI interface.
///
/// This function stops the existing live measurement subscription, waits for a random duration between 10 and 120 seconds,
/// and then attempts to reconnect. It checks periodically if the application should quit during the waiting period.
///
/// # Arguments
///
/// * `access_config` - A reference to the access configuration required for reconnection.
/// * `subscription` - The existing live measurement subscription that needs to be stopped.
/// * `app_state` - An atomic reference-counted mutex containing the application state, used to check if a shutdown is requested.
///
/// # Returns
///
/// If successful, returns a new live measurement subscription. Otherwise, returns an error indicating the reason for failure.
///
/// # Errors
///
/// This function can return errors in the following scenarios:
/// - If there is an issue stopping the existing subscription.
/// - If the application state indicates a shutdown request during the waiting period.
/// - If there is an error establishing a new live measurement connection.
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

    let base_delay = 10; // Base delay in seconds
    let max_jitter = 110; // Maximum jitter in seconds

    let jitter = rand::thread_rng().gen_range(0..=max_jitter);
    let number_of_seconds = base_delay + jitter;
    info!(target: "tibberator.app", "Connection lost, waiting for {}s before reconnecting.", number_of_seconds);

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
/// Returns a `Result<Option<Box<Subscription<StreamingOperation<LiveMeasurement>>>>, Box<dyn std::error::Error + Send + Sync>>`:
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
    update_current_energy_price(&config.access, app_state.lock().unwrap().deref_mut()).await?;

    // fetch an estimate for the daily fee
    let estimated_daily_fee_result = estimate_daily_fees(&config.access).await;
    app_state.lock().unwrap().estimated_daily_fees = match estimated_daily_fee_result {
        Ok(estimated_daily_fee) => estimated_daily_fee,
        Err(err) => {
            error!(target: "tibberator.mainloop", "Failed to fetch daily fee estimate: {:?}", err);
            return Err(Box::<dyn std::error::Error + Send + Sync>::from(
                std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid Data"),
            ));
        }
    };

    // Fetch display data based on display mode
    {
        let mut state = app_state.lock().unwrap();
        let current_display_mode = state.display_mode.to_owned();

        match tibberator::tibber::fetch_display_data(
            &config.access,
            &current_display_mode,
            &state.estimated_daily_fees,
        )
        .await
        {
            Ok(Some((data, label, data_time))) => {
                // Update app state with bar graph data
                state
                    .cached_bar_graph
                    .insert(current_display_mode, (data, label.to_string(), data_time));
            }
            Ok(None) => {
                info!(target: "tibberator.mainloop", "No display data available");
            }
            Err(err) => {
                error!(target: "tibberator.mainloop", "Failed to fetch display data: {:?}", err);
            }
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
        // Check if data needs refresh
        {
            let cloned_app_state = app_state.clone();
            let mut state = cloned_app_state.lock().unwrap();
            update_current_energy_price(&config.access, state.deref_mut()).await?;

            let current_display_mode = state.display_mode.to_owned();

            if state.data_needs_refresh || cache_expired(&state) {
                info!(target: "tibberator.mainloop", "Fetching new display data for {:?}", state.display_mode);
                match tibberator::tibber::fetch_display_data(
                    &config.access,
                    &state.display_mode,
                    &state.estimated_daily_fees,
                )
                .await
                {
                    Ok(Some((data, label, data_time))) => {
                        state
                            .cached_bar_graph
                            .insert(current_display_mode, (data, label.to_string(), data_time));
                    }
                    Ok(None) => {
                        info!(target: "tibberator.mainloop", "No display data available");
                        let data_vector = Vec::new();
                        state.cached_bar_graph.insert(
                            current_display_mode,
                            (
                                data_vector,
                                String::from("No data available"),
                                Local::now().fixed_offset() + chrono::Duration::days(1),
                            ),
                        );
                    }
                    Err(err) => {
                        error!(target: "tibberator.mainloop", "Failed to fetch display data: {:?}", err);
                    }
                }
                state.data_needs_refresh = false;
            }
        }

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

/// Updates the current energy price information in the application state.
///
/// This asynchronous function fetches the latest energy price information from Tibber using the provided access configuration.
/// If the fetched price information is different from the currently stored one, it marks the data as needing refresh and updates the stored price information.
///
/// # Arguments
/// * `access_config` - A reference to the AccessConfig containing the necessary credentials and settings to access Tibber's API.
/// * `app_state` - A mutable reference to the AppState where the fetched price information will be stored or updated.
///
/// # Returns
/// * `Ok(())` - If the price information is successfully fetched and updated (or no update was needed).
/// * `Err(Box<LoopEndingError::InvalidData>)` - If there was an error fetching the price information, logging the error and returning an invalid data error.
///
/// # Errors
/// This function will return an error if:
/// * The API request to fetch the price information fails.
async fn update_current_energy_price(
    access_config: &AccessConfig,
    app_state: &mut AppState,
) -> Result<(), Box<LoopEndingError>> {
    match tibberator::tibber::update_current_energy_price_info(
        &access_config,
        app_state.price_info.clone(),
    )
    .await
    {
        Ok(price_info) => {
            if app_state
                .price_info
                .as_ref()
                .map_or(true, |current| current != &price_info)
            {
                app_state.data_needs_refresh = true;
                app_state.price_info = Some(price_info);
            }
            Ok(())
        }
        Err(err) => {
            error!(target: "tibberator.mainloop", "Failed to fetch price info: {:?}", err);
            Err(Box::new(LoopEndingError::InvalidData))
        }
    }
}

/// Process data from a subscription and call appropriate handlers.
///
/// This function asynchronously processes incoming data from a `LiveMeasurementSubscription`.
/// It uses the provided `data_handler` to handle valid power measurements, `error_handler`
/// for invalid data scenarios, and `reconnect_handler` for reconnection attempts. The function
/// also checks for user shutdown requests or reconnect timeouts to manage the loop's termination.
///
/// # Parameters:
/// - `config`: A reference to the configuration settings which include access timeout values.
/// - `app_state`: An arc-wrapped mutex containing the application state, used to check if a shutdown is requested.
/// - `subscription`: A mutable reference to the live measurement subscription from which data is received.
/// - `data_handler`: A function that takes a `LiveMeasurementLiveMeasurement` and processes it.
/// - `error_handler`: A function that takes an error message as a string and handles errors.
/// - `reconnect_handler`: A function that handles the reconnection logic when necessary.
///
/// # Returns:
/// This function returns a `Result` indicating success or an error. The error can be one of several
/// types encapsulated in a `Box<LoopEndingError>`, including `Shutdown`, `Reconnect`, or `InvalidData`.
///
/// # Errors:
/// - `LoopEndingError::Shutdown`: If the user requests shutdown through the application state.
/// - `LoopEndingError::Reconnect`: If a reconnect timeout occurs or is manually triggered.
/// - `LoopEndingError::InvalidData`: If the received data is invalid.
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
    app_state: Arc<Mutex<AppState>>,
) -> Result<Option<Box<LiveMeasurementSubscription>>, Box<dyn std::error::Error + Send + Sync>> {
    let mut subscription = Box::new(connect_live_measurement(&config.access).await);

    write!(stdout(), "Waiting for data ...").unwrap();
    stdout().flush().unwrap();

    loop {
        let final_result = loop_for_data(&config, subscription.as_mut(), app_state.clone()).await;
        match final_result {
            Ok(()) => break,
            Err(error) => match error.downcast_ref::<LoopEndingError>() {
                Some(LoopEndingError::Reconnect) | Some(LoopEndingError::ConnectionTimeout) => {
                    let subscription_result =
                        handle_reconnect(&config.access, subscription, app_state.clone()).await;

                    match subscription_result {
                        Ok(res) => subscription = Box::new(res),
                        Err(LoopEndingError::Shutdown) => {
                            return Ok(None);
                        }
                        Err(error) => {
                            eprintln!("Unexpected error: {:?}", error);
                            return Err(Box::new(error) as Box<dyn std::error::Error + Send + Sync>);
                        }
                    }
                }
                Some(LoopEndingError::InvalidData) => {
                    return Err(Box::new(LoopEndingError::InvalidData)
                        as Box<dyn std::error::Error + Send + Sync>)
                }
                _ => {
                    eprintln!("Unexpected error: {:?}", error);
                    return Err(Box::<dyn std::error::Error + Send + Sync>::from(
                        std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid data"),
                    ));
                }
            },
        }
    }

    Ok(Some(subscription))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use tibberator::tibber::output::DisplayMode;
    use tokio::time::sleep;

    fn get_test_config() -> Config {
        let current_dir = env::current_dir().expect("Failed to get current directory.");
        let filename = "tests/test-config.toml";
        let path = current_dir.join(filename);
        confy::load_path(path).expect("Config file not found.")
    }

    fn create_app_state() -> Arc<Mutex<AppState>> {
        Arc::new(Mutex::new(AppState {
            should_quit: false,
            measurement: None,
            price_info: None,
            cached_bar_graph: HashMap::new(),
            estimated_daily_fees: None,
            display_mode: DisplayMode::Prices,
            status: String::from("Waiting for data..."),
            data_needs_refresh: false,
        }))
    }

    #[tokio::test]
    async fn test_subscription_loop() {
        let config = get_test_config();

        let app_state = create_app_state();

        let subscription = subscription_loop(config, app_state.clone());
        sleep(time::Duration::from_secs(5)).await;
        app_state.lock().unwrap().should_quit = true;
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
