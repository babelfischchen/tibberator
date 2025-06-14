use std::{
    collections::HashMap,
    fs::{create_dir_all, File},
    io::{stdin, stdout, Read, Write},
    ops::DerefMut,
    process::ExitCode,
    sync::{Arc, Mutex},
    thread::sleep,
    time::Instant,
};

use chrono::Local;
use clap_v3::{App, Arg, ArgMatches};

use confy;

use dirs::home_dir;
use exitcode;
use futures::{executor::block_on, stream::StreamExt};

use log::{debug, error, info, trace, warn, SetLoggerError};
use rand::Rng;
use tokio::time;

use tibberator::{
    html_logger::{HtmlLogger, LogConfig},
    tibber::{
        cache_expired, connect_live_measurement, estimate_daily_fees, get_home_ids, loop_for_data,
        output::{self, DisplayMode, GuiMode},
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

/// Initializes the HTML logger with the specified file path and log configuration.
///
/// # Arguments
/// * `file_path` - The path to the log file to be created.
/// * `log_config` - The configuration settings for the logger.
///
/// # Returns
/// * `Ok(())` if initialization is successful.
/// * `Err(SetLoggerError)` if there's an error setting the logger.
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
        data_needs_refresh: true,
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
    while !app_state.lock().unwrap().should_quit && !subscription_handle.is_finished() {
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

/// Creates and configures the command-line argument parser using `clap_v3`.
///
/// Defines the application name, version, author, description, and expected arguments.
///
/// # Returns
/// * `ArgMatches` - The parsed command-line arguments.
fn get_matcher() -> ArgMatches {
    App::new("Tibberator")
        .version("0.5.1")
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

/// Handles the main subscription loop for receiving live measurement data with TUI updates.
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
    // Stop the existing subscription with error handling
    match subscription.stop().await {
        Ok(_) => (),
        Err(error) => {
            match error {
                graphql_ws_client::Error::Send(err_string) => {
                    // Channel is closed, subscription already stopped – proceed
                    info!(target: "tibberator.app", "Subscription already stopped, proceeding to reconnect: {}", err_string);
                }
                _ => {
                    error!(target: "tibberator.app", "Failed to stop subscription: {:?}", error);
                    return Err(LoopEndingError::InvalidData);
                }
            }
        }
    };

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

    // fetch an estimate for the daily fee
    update_daily_fees(&config.access, &app_state).await?;

    // Fetch display data based on display mode
    update_display_data(&config.access, &app_state).await?;

    // Create a custom data handler that updates the app state
    let data_handler = create_data_handler(app_state.clone());

    // Create a custom error handler that updates the app state
    let error_handler = create_error_handler(app_state.clone());

    // Create a custom reconnect handler that updates the app state
    let reconnect_handler = create_reconnect_handler(app_state.clone());

    let mut last_value_received = Instant::now();
    let poll_timeout = std::time::Duration::from_secs(15);

    // Main subscription loop
    while !app_state.lock().unwrap().should_quit {
        // Check for reconnect timeout
        match refresh_subscription_if_outdated(
            &config.access,
            subscription,
            &mut last_value_received,
            &reconnect_handler,
            &app_state,
        )
        .await?
        {
            Some(new_subscription) => {
                subscription = new_subscription;
            }
            None => {
                info!(target: "tibberator.mainloop", "Shutdown during reconnect.");
                return Ok(None); // User requested shutdown during reconnection
            }
        };

        // Check if data needs refresh
        update_display_data(&config.access, &app_state).await?;

        // Poll data using the dedicated function
        // Pass ownership of subscription to poll_data and reassign the returned Box
        match poll_data(
            &config.access,
            subscription, // Pass ownership
            &mut last_value_received,
            poll_timeout,
            &data_handler,
            &error_handler,
            &reconnect_handler,
            app_state.clone(),
        )
        .await
        {
            Ok(new_subscription) => {
                subscription = new_subscription; // Assign back the potentially new subscription
            }
            Err(err) => {
                // Check if the error is a Shutdown signal
                if let Some(LoopEndingError::Shutdown) = err.downcast_ref::<LoopEndingError>() {
                    info!(target: "tibberator.mainloop", "Shutdown signal received during poll_data.");
                    return Ok(None); // User requested shutdown
                } else {
                    // Handle other critical errors from poll_data
                    error!(target: "tibberator.mainloop", "Critical error during data polling: {:?}", err);
                    // Propagate the error out of subscription_loop_tui
                    return Err(err);
                }
            }
        }
        // Small sleep to prevent tight loop if polling is very fast and no data arrives
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    info!(target: "tibberator.mainloop", "Exiting subscription loop.");
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

/// Fetches the estimated daily fees and updates the application state.
///
/// # Arguments
/// * `access_config` - Configuration required to access the Tibber API.
/// * `app_state` - Shared application state to update with the fee estimate.
///
/// # Returns
/// * `Ok(())` on successful update.
/// * `Err` if fetching the fee estimate fails.
async fn update_daily_fees(
    access_config: &AccessConfig,
    app_state: &Arc<Mutex<AppState>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let estimated_daily_fee_result = estimate_daily_fees(access_config).await;
    let mut state = app_state.lock().unwrap();
    state.estimated_daily_fees = match estimated_daily_fee_result {
        Ok(estimated_daily_fee) => estimated_daily_fee,
        Err(err) => {
            error!(target: "tibberator.mainloop", "Failed to fetch daily fee estimate: {:?}", err);
            return Err(Box::<dyn std::error::Error + Send + Sync>::from(
                std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid Data"),
            ));
        }
    };
    Ok(())
}

/// Creates a closure that handles incoming live measurement data.
///
/// The returned closure updates the `measurement` and `status` fields in the shared `AppState`.
///
/// # Arguments
/// * `app_state` - The shared application state (`Arc<Mutex<AppState>>`).
///
/// # Returns
/// * A `Box` containing the data handling closure.
fn create_data_handler(
    app_state: Arc<Mutex<AppState>>,
) -> Box<dyn Fn(tibberator::tibber::live_measurement::LiveMeasurementLiveMeasurement)> {
    Box::new(
        move |data: tibberator::tibber::live_measurement::LiveMeasurementLiveMeasurement| {
            let mut state = app_state.lock().unwrap();
            state.measurement = Some(data);
            state.status = String::from("Connected");
        },
    )
}

/// Creates a closure that handles errors occurring during data fetching or processing.
///
/// The returned closure logs the error and updates the `status` field in the shared `AppState`.
///
/// # Arguments
/// * `app_state` - The shared application state (`Arc<Mutex<AppState>>`).
///
/// # Returns
/// * A `Box` containing the error handling closure.
fn create_error_handler(app_state: Arc<Mutex<AppState>>) -> Box<dyn Fn(&str)> {
    Box::new(move |error: &str| {
        error!(target: "tibberator.mainloop", "Error occured: {}", error);
        let mut state = app_state.lock().unwrap();
        state.status = format!("Error: {}", error);
    })
}

/// Creates a closure that handles the initiation of a reconnection attempt.
///
/// The returned closure logs a warning and updates the `status` field in the shared `AppState`
/// to indicate that a reconnection is in progress.
///
/// # Arguments
/// * `app_state` - The shared application state (`Arc<Mutex<AppState>>`).
///
/// # Returns
/// * A `Box` containing the reconnect handling closure.
fn create_reconnect_handler(app_state: Arc<Mutex<AppState>>) -> Box<dyn Fn()> {
    Box::new(move || {
        warn!(target: "tibberator.mainloop", "Reconnecting...");
        let mut state = app_state.lock().unwrap();
        state.status = String::from("Reconnecting...");
    })
}

/// Checks if the subscription needs refreshing due to inactivity and handles reconnection.
///
/// If the time since the last received value exceeds the `reconnect_timeout`, it attempts
/// to reconnect using `handle_reconnect_tui`.
///
/// # Arguments
/// * `access_config` - Configuration for accessing Tibber API.
/// * `subscription` - The current live measurement subscription.
/// * `last_value_received` - Timestamp of the last successfully received data.
/// * `reconnect_handler` - Closure to execute when reconnection starts.
/// * `app_state` - Shared application state.
///
/// # Returns
/// * `Ok(Some(Box<LiveMeasurementSubscription>))` - The original or a new subscription if successful.
/// * `Ok(None)` - If shutdown was requested during reconnection.
/// * `Err` - If reconnection failed.
async fn refresh_subscription_if_outdated(
    access_config: &AccessConfig,
    mut subscription: Box<LiveMeasurementSubscription>,
    last_value_received: &mut Instant,
    reconnect_handler: &Box<dyn Fn()>,
    app_state: &Arc<Mutex<AppState>>,
) -> Result<Option<Box<LiveMeasurementSubscription>>, Box<dyn std::error::Error + Send + Sync>> {
    if last_value_received.elapsed().as_secs() > access_config.reconnect_timeout {
        warn!(target: "tibberator.mainloop", "Reconnect timeout reached.");
        reconnect_handler();
        match handle_reconnect_tui(access_config, subscription, app_state.clone()).await {
            Ok(new_subscription) => {
                subscription = Box::new(new_subscription);
                *last_value_received = Instant::now(); // Reset timer after successful reconnect
                let mut state = app_state.lock().unwrap();
                state.status = String::from("Reconnected, waiting for data...");
            }
            Err(LoopEndingError::Shutdown) => {
                info!(target: "tibberator.mainloop", "Shutdown during reconnect.");
                return Ok(None);
            }
            Err(err) => {
                error!(target: "tibberator.mainloop", "Reconnection failed: {:?}", err);
                return Err(Box::new(err));
            }
        }
    }
    Ok(Some(subscription))
}

/// Updates the display data (prices or consumption) based on the current display mode.
///
/// Fetches new data if the cache is expired or if `data_needs_refresh` is true.
/// Updates the `cached_bar_graph` and resets `data_needs_refresh` in the `AppState`.
/// Also ensures the current energy price is up-to-date.
///
/// # Arguments
/// * `access_config` - Configuration for accessing Tibber API.
/// * `app_state` - Shared application state.
///
/// # Returns
/// * `Ok(())` on successful update or if no update was needed.
/// * `Err` if fetching data fails.
async fn update_display_data(
    access_config: &AccessConfig,
    app_state: &Arc<Mutex<AppState>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cloned_app_state = app_state.clone(); // Clone needed for async block
    let mut state = cloned_app_state.lock().unwrap();
    update_current_energy_price(access_config, state.deref_mut()).await?;

    let current_display_mode = state.display_mode.to_owned();

    if state.data_needs_refresh || cache_expired(&state) {
        info!(target: "tibberator.mainloop", "Fetching new display data for {:?}", state.display_mode);

        match current_display_mode {
            DisplayMode::Prices | DisplayMode::PricesTomorrow => {
                match tibberator::tibber::fetch_prices_display_data(access_config).await {
                    Ok(Some(((today, tomorrow), label, data_time))) => {
                        state.cached_bar_graph.insert(
                            DisplayMode::Prices,
                            (today, label.to_string() + " - Today", data_time),
                        );
                        state.cached_bar_graph.insert(
                            DisplayMode::PricesTomorrow,
                            (tomorrow, label.to_string() + " - Tomorrow", data_time),
                        );
                    }
                    Ok(None) => {
                        info!(target: "tibberator.mainloop", "No display data available");
                        let data_vector_today = Vec::new();
                        let data_vector_tomorrow = Vec::new();
                        state.cached_bar_graph.insert(
                            DisplayMode::Prices,
                            (
                                data_vector_today,
                                String::from("No data available"),
                                Local::now().fixed_offset() + chrono::Duration::days(1),
                            ),
                        );
                        state.cached_bar_graph.insert(
                            DisplayMode::Prices,
                            (
                                data_vector_tomorrow,
                                String::from("No data available"),
                                Local::now().fixed_offset() + chrono::Duration::days(1),
                            ),
                        );
                    }
                    Err(err) => {
                        error!(target: "tibberator.mainloop", "Failed to fetch display data: {:?}", err);
                    }
                }
            }
            _ => match tibberator::tibber::fetch_display_data(
                access_config,
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
            },
        }
        state.data_needs_refresh = false;
    }
    Ok(())
}

/// Polls the subscription for new data and handles the outcome.
///
/// This function attempts to get the next item from the subscription stream using `process_subscription_data`.
/// It handles successful data reception, stream ending (triggering reconnect), timeouts, and invalid data (triggering reconnect).
///
/// # Arguments
/// * `access_config` - Configuration for accessing Tibber API.
/// * `subscription` - The current live measurement subscription (ownership is taken and returned).
/// * `last_value_received` - Timestamp of the last successfully received data (updated on success).
/// * `poll_timeout` - Duration to wait for data before timing out.
/// * `data_handler` - Closure to process valid received data.
/// * `error_handler` - Closure to handle errors during polling or reconnection.
/// * `reconnect_handler` - Closure to execute when reconnection starts.
/// * `app_state` - Shared application state.
///
/// # Returns
/// * `Ok(Box<LiveMeasurementSubscription>)` - The subscription (potentially new after reconnect) on success or timeout.
/// * `Err(Box<dyn std::error::Error + Send + Sync>)` - If a critical error occurs (e.g., shutdown, failed reconnect).
async fn poll_data(
    access_config: &AccessConfig,
    mut subscription: Box<LiveMeasurementSubscription>, // Take ownership
    last_value_received: &mut Instant,
    poll_timeout: std::time::Duration,
    data_handler: &Box<
        dyn Fn(tibberator::tibber::live_measurement::LiveMeasurementLiveMeasurement),
    >,
    error_handler: &Box<dyn Fn(&str)>,
    reconnect_handler: &Box<dyn Fn()>,
    app_state: Arc<Mutex<AppState>>,
) -> Result<Box<LiveMeasurementSubscription>, Box<dyn std::error::Error + Send + Sync>> {
    let result = process_subscription_data(
        subscription.as_mut(),
        poll_timeout,
        data_handler,
        error_handler,
    )
    .await;

    match result {
        Ok(Some(())) => {
            // Data received successfully
            *last_value_received = Instant::now(); // Reset the timer
            trace!(target: "tibberator.mainloop", "Data processed successfully");
        }
        Ok(None) => {
            // Stream ended normally, treat as reconnect scenario
            warn!(target: "tibberator.mainloop", "Subscription stream ended unexpectedly. Reconnecting...");
            reconnect_handler();
            match handle_reconnect_tui(access_config, subscription, app_state.clone()).await {
                Ok(new_subscription) => {
                    subscription = Box::new(new_subscription); // Assign back the new owned Box
                    *last_value_received = Instant::now();
                    let mut state = app_state.lock().unwrap();
                    state.status = String::from("Reconnected, waiting for data...");
                }
                Err(LoopEndingError::Shutdown) => return Err(Box::new(LoopEndingError::Shutdown)), // Propagate shutdown error
                Err(err) => {
                    error_handler(&format!("Reconnection failed after stream end: {:?}", err));
                    return Err(Box::new(err)); // Propagate other errors
                }
            }
        }
        Err(LoopEndingError::ConnectionTimeout) => {
            // Poll timed out, just continue loop. The elapsed time check will handle actual reconnects.
            trace!(target: "tibberator.mainloop", "Poll timed out, continuing loop.");
        }
        Err(LoopEndingError::InvalidData) => {
            // Invalid data received, trigger reconnect
            warn!(target: "tibberator.mainloop", "Invalid data received. Reconnecting...");
            reconnect_handler();
            match handle_reconnect_tui(access_config, subscription, app_state.clone()).await {
                // Pass ownership
                Ok(new_subscription) => {
                    subscription = Box::new(new_subscription); // Assign back the new owned Box
                    *last_value_received = Instant::now();
                    let mut state = app_state.lock().unwrap();
                    state.status = String::from("Reconnected after invalid data, waiting...");
                }
                Err(LoopEndingError::Shutdown) => return Err(Box::new(LoopEndingError::Shutdown)), // Propagate shutdown error
                Err(err) => {
                    error_handler(&format!(
                        "Reconnection failed after invalid data: {:?}",
                        err
                    ));
                    return Err(Box::new(err));
                }
            }
        }
        // Note: process_subscription_data should ideally not return other errors,
        // but if it does, we treat it as critical.
        Err(other_error) => {
            error!(target: "tibberator.mainloop", "Unexpected error from process_subscription_data: {:?}", other_error);
            error_handler(&format!("Unexpected error: {:?}", other_error));
            // Treat unexpected errors as critical and propagate them
            return Err(Box::new(other_error)); // Propagate other critical errors
        }
    }
    Ok(subscription) // Return the subscription Box on success
}

/// Processes incoming subscription data, handling various scenarios such as successful reception of data,
/// errors in the stream, and timeouts.
///
/// This function takes a mutable reference to a `LiveMeasurementSubscription`, a timeout duration for polling,
/// a data handler closure to process valid data, and an error handler closure to handle any issues encountered during processing.
///
/// # Arguments:
/// * `subscription`: A mutable reference to the live measurement subscription.
/// * `timeout_duration`: The duration after which the poll times out if no data is received.
/// * `data_handler`: A closure that processes valid live measurement data.
/// * `error_handler`: A closure that handles errors encountered during processing.
///
/// # Returns:
/// - `Ok(Some(()))`: If valid data was received and processed successfully.
/// - `Ok(None)`: If the subscription stream ended normally without any errors.
/// - `Err(LoopEndingError::ConnectionTimeout)`: If the poll times out.
/// - `Err(LoopEndingError::InvalidData)`: If invalid data is received or a stream error occurred.
///
/// # Errors:
/// - `LoopEndingError::ConnectionTimeout`: If the poll times out.
/// - `LoopEndingError::InvalidData`: If invalid data is received or a stream error occurs.
async fn process_subscription_data<F, E>(
    subscription: &mut LiveMeasurementSubscription,
    timeout_duration: std::time::Duration,
    data_handler: &F,
    error_handler: &E,
) -> Result<Option<()>, LoopEndingError>
where
    F: Fn(tibberator::tibber::live_measurement::LiveMeasurementLiveMeasurement),
    E: Fn(&str),
{
    match tokio::time::timeout(timeout_duration, subscription.next()).await {
        Ok(Some(Ok(response))) => {
            // Successfully received a response
            if let Some(data) = response.data {
                if let Some(measurement) = data.live_measurement {
                    debug!(target: "tibberator.mainloop", "Received power measurement: {} W", measurement.power);
                    data_handler(measurement);
                    Ok(Some(())) // Indicate data was received
                } else {
                    error!(target: "tibberator.mainloop", "Live measurement data missing in response.");
                    error_handler("Live measurement data missing");
                    Err(LoopEndingError::InvalidData)
                }
            } else {
                error!(target: "tibberator.mainloop", "No data field in response.");
                error_handler("No data field in response");
                Err(LoopEndingError::InvalidData)
            }
        }
        Ok(Some(Err(e))) => {
            // GraphQL client error during streaming
            error!(target: "tibberator.mainloop", "GraphQL client error: {:?}", e);
            error_handler(&format!("GraphQL client error: {}", e));
            Err(LoopEndingError::InvalidData) // Treat client errors as invalid data for now
        }
        Ok(None) => {
            // Stream ended normally
            info!(target: "tibberator.mainloop", "Subscription stream ended.");
            Ok(None) // Indicate stream ended
        }
        Err(_) => {
            // Timeout occurred
            trace!(target: "tibberator.mainloop", "Subscription poll timed out.");
            Err(LoopEndingError::ConnectionTimeout)
        }
    }
}

/// Handles the main subscription loop for the simple (non-TUI) mode.
///
/// This function connects to the Tibber live measurement stream and processes incoming data.
/// It includes logic for handling reconnections based on errors or timeouts.
/// Kept primarily for compatibility with older versions or specific test cases.
///
/// # Arguments
/// * `config` - The application configuration.
/// * `app_state` - Shared application state, used for checking shutdown signals.
///
/// # Returns
/// * `Ok(Some(Box<LiveMeasurementSubscription>))` - The final subscription object if the loop completed normally.
/// * `Ok(None)` - If a shutdown was requested during reconnection.
/// * `Err` - If an unrecoverable error occurred.
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
