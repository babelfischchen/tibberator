use std::{
    io::{stdin, stdout, Read, Write},
    process::ExitCode,
    sync::mpsc::{self, Receiver},
};

use chrono::Local;
use clap_v3::{App, Arg, ArgMatches};

use confy;

use crossterm::{
    cursor, execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen},
};
use ctrlc::set_handler;
use dirs::home_dir;
use exitcode;
use futures::executor::block_on;
use log::{error, info, SetLoggerError};
use rand::Rng;
use std::fs::{create_dir_all, File};
use tokio::time;

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

    execute!(stdout(), EnterAlternateScreen, cursor::Hide).unwrap();

    let (sender, receiver) = mpsc::channel();
    set_handler(move || {
        sender.send(true).unwrap();
    })
    .expect("Error setting CTRL+C handler");

    let subscription = block_on(subscription_loop(config, receiver));

    execute!(stdout(), LeaveAlternateScreen).unwrap();

    let exitcode = match subscription {
        Ok(result) => match result {
            Some(value) => {
                let stop_result = block_on(value.stop());
                match stop_result {
                    Ok(()) => {
                        info!(target: "tibberator.app", "Application stopped.");
                        ExitCode::SUCCESS
                    }
                    Err(error) => {
                        println!("{:?}", error);
                        error!(target: "tibberator.app", "Exiting application due to error: {:?}", error);
                        ExitCode::FAILURE
                    }
                }
            }
            _ => ExitCode::FAILURE,
        },
        Err(error) => {
            println!("{:?}", error);
            println!("Press Enter to continue...");

            let _ = stdin().read(&mut [0u8]).unwrap();
            ExitCode::FAILURE
        }
    };

    log::logger().flush();

    exitcode
}

fn get_matcher() -> ArgMatches {
    App::new("Tibberator")
        .version("0.1.0")
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
    receiver: &Receiver<bool>,
) -> Result<LiveMeasurementSubscription, LoopEndingError> {
    if let Err(error) = subscription.stop().await {
        eprintln!("{:?}", error);
        std::process::exit(exitcode::PROTOCOL)
    };

    let number_of_seconds = rand::thread_rng().gen_range(1..=60);
    println!(
        "\nConnection lost, waiting for {}s before reconnecting.",
        number_of_seconds
    );

    for _ in 0..=number_of_seconds {
        if check_user_shutdown(&receiver) == true {
            return Err(LoopEndingError::Shutdown);
        }
        std::thread::sleep(time::Duration::from_secs(1));
    }

    Ok(connect_live_measurement(&access_config).await)
}

/// Handles the main subscription loop for receiving live measurement data.
///
/// This asynchronous function takes the following parameters:
/// - `config`: A `Config` struct containing configuration details.
/// - `receiver`: A `Receiver<bool>` used for checking user shutdown signals.
///
/// The function performs the following steps:
/// 1. Initializes a subscription by connecting to live measurement data using `connect_live_measurement(&config.access).await`.
/// 2. Prints a message indicating that it is waiting for data.
/// 3. Enters a loop to continuously process data:
///    - Calls `loop_for_data(&config, subscription.as_mut(), &receiver).await`.
///    - Handles different error scenarios:
///      - If the loop ends due to successful data processing, breaks out of the loop.
///      - If the loop ends due to a reconnect signal, calls `handle_reconnect(&config.access, subscription, &receiver).await`.
///        - If reconnection is successful, updates the subscription.
///        - If the user initiates a shutdown during reconnection, returns `Ok(None)`.
///        - If an unexpected error occurs during reconnection, prints an error message and returns an error.
///      - If the loop ends due to invalid data, returns the error.
///      - If an unexpected error occurs, prints an error message and returns an error.
///
/// # Errors
/// Returns a `Result<Option<Box<Subscription<StreamingOperation<LiveMeasurement>>>>, Box<dyn std::error::Error>>`:
/// - `Ok(Some(subscription))`: Successfully processed data, returning the updated subscription.
/// - `Ok(None)`: User initiated shutdown during reconnection.
/// - `Err(error)`: An error occurred during data processing or reconnection.
///
async fn subscription_loop(
    config: Config,
    receiver: Receiver<bool>,
) -> Result<Option<Box<LiveMeasurementSubscription>>, Box<dyn std::error::Error>> {
    let mut subscription = Box::new(connect_live_measurement(&config.access).await);

    write!(stdout(), "Waiting for data ...").unwrap();
    stdout().flush().unwrap();

    loop {
        let final_result = loop_for_data(&config, subscription.as_mut(), &receiver).await;
        match final_result {
            Ok(()) => break,
            Err(error) => match error.downcast_ref::<LoopEndingError>() {
                Some(LoopEndingError::Reconnect) => {
                    let subscription_result =
                        handle_reconnect(&config.access, subscription, &receiver).await;

                    match subscription_result {
                        Ok(res) => subscription = Box::new(res),
                        Err(LoopEndingError::Shutdown) => {
                            return Ok(None);
                        }
                        Err(error) => {
                            eprintln!("Unexpected error: {:?}", error);
                            return Err(Box::new(error));
                        }
                    }
                }
                Some(LoopEndingError::InvalidData) => return Err(error),
                _ => {
                    eprintln!("Unexpected error: {:?}", error);
                    return Err(error);
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
        let config = get_test_config();
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
