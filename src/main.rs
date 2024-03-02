use std::{
    io::{self, stdin, stdout, Read, Write},
    sync::mpsc::{self, Receiver, RecvTimeoutError},
    time::Duration,
};

use clap_v3::{App, Arg, ArgMatches};

use chrono::DateTime;

use confy;

use crossterm::{
    cursor, execute, queue,
    style::Stylize,
    terminal::{Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use ctrlc::set_handler;
use exitcode;
use futures::{executor::block_on, stream::StreamExt};
use graphql_ws_client::{graphql::StreamingOperation, Subscription};
use rand::Rng;
use tokio::time;

use tibberator::tibber::{
    get_live_measurement, live_measurement::LiveMeasurementLiveMeasurement, AccessConfig,
    LiveMeasurement,
};

fn get_config() -> Result<AccessConfig, confy::ConfyError> {
    let matches = get_matcher();
    let app_name = "Tibberator";
    let config_name = "config";

    if let Some(access_token) = matches.value_of("token") {
        let mut config: AccessConfig = confy::load("Tibberator", "config")?;
        config.token = String::from(access_token);
        confy::store(app_name, config_name, config)?;
    };

    if let Some(home_id) = matches.value_of("home_id") {
        let mut config: AccessConfig = confy::load("Tibberator", "config")?;
        config.home_id = String::from(home_id);
        confy::store(app_name, config_name, config)?;
    }

    let config: AccessConfig = confy::load(app_name, config_name)?;
    Ok(config)
}

#[tokio::main]
async fn main() {
    let config = get_config().expect("Config file must be loaded.");

    execute!(stdout(), EnterAlternateScreen, cursor::Hide).unwrap();

    let (sender, receiver) = mpsc::channel();
    set_handler(move || {
        sender.send(true).unwrap();
    })
    .expect("Error setting CTRL+C handler");

    let subscription = block_on(subscription_loop(config, receiver));

    execute!(stdout(), LeaveAlternateScreen).unwrap();

    match subscription {
        Ok(result) => match result {
            Some(value) => {
                let stop_result = block_on(value.stop());
                match stop_result {
                    Ok(()) => std::process::exit(exitcode::OK),
                    Err(error) => {
                        println!("{:?}", error);
                        std::process::exit(exitcode::PROTOCOL)
                    }
                };
            }
            _ => {}
        },
        Err(error) => {
            println!("{:?}", error);
            println!("Press Enter to continue...");

            let _ = stdin().read(&mut [0u8]).unwrap();
            std::process::exit(exitcode::PROTOCOL)
        }
    };
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

fn print_screen(data: LiveMeasurementLiveMeasurement) {
    let timestamp = DateTime::parse_from_str(&data.timestamp, "%+").unwrap();
    let str_timestamp = timestamp.format("%H:%M:%S");

    let mut line_number = 1;

    queue!(
        stdout(),
        Clear(ClearType::All),
        cursor::MoveTo(1, line_number)
    )
    .unwrap();

    let mut move_cursor = |increase: u16| {
        line_number += increase;
        queue!(stdout(), cursor::MoveTo(1, line_number)).unwrap();
    };

    let power_production = data.power_production.unwrap_or(0.);

    // time
    write!(stdout(), "Time:").unwrap();
    move_cursor(1);
    write!(stdout(), "{}", str_timestamp).unwrap();
    move_cursor(2);

    // current power
    if power_production == 0. {
        write!(stdout(), "{}", "Current Power consumption:".red()).unwrap();
        move_cursor(1);
        write!(stdout(), "{:.1} W", data.power).unwrap();
    }
    // current production
    else {
        write!(stdout(), "{}", "Current Power production:".green()).unwrap();
        move_cursor(1);
        write!(stdout(), "{:.1} W", power_production).unwrap();
    }
    // cost today
    move_cursor(2);
    write!(stdout(), "Cost today:").unwrap();
    move_cursor(1);
    write!(
        stdout(),
        "{:.2} {}",
        data.accumulated_cost.unwrap_or(-1.),
        data.currency.unwrap_or(String::from("None"))
    )
    .unwrap();

    // consumption today
    move_cursor(2);
    write!(stdout(), "Consumption today:").unwrap();
    move_cursor(1);
    write!(stdout(), "{:.3} kWh", data.accumulated_consumption).unwrap();

    // production today
    move_cursor(2);
    write!(stdout(), "Production today:").unwrap();
    move_cursor(1);
    write!(stdout(), "{:.3} kWh", data.accumulated_production).unwrap();
    execute!(stdout(), cursor::Hide).unwrap();

    stdout().flush().unwrap();
}

fn check_user_shutdown(receiver: &Receiver<bool>) -> bool {
    let received_value = receiver.recv_timeout(Duration::from_millis(100));
    match received_value {
        Ok(value) => value,
        Err(error) => match error {
            RecvTimeoutError::Timeout => false,
            RecvTimeoutError::Disconnected => {
                println!("{:?}", error);
                true
            }
        },
    }
}

async fn connect_live_measurement(
    config: &AccessConfig,
) -> Option<Subscription<StreamingOperation<LiveMeasurement>>> {
    let subscription = get_live_measurement(&config).await;

    match subscription {
        Ok(result) => {
            println!("Connection established");
            Some(result)
        }
        Err(error) => {
            println!("{:?}", error);
            std::process::exit(exitcode::PROTOCOL);
        }
    }
}

async fn loop_for_data(
    config: &AccessConfig,
    subscription: &mut Subscription<StreamingOperation<LiveMeasurement>>,
    receiver: &Receiver<bool>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut loop_counter = 0;
    loop {
        let next_data = time::timeout(Duration::from_secs(1), subscription.next()).await;
        match next_data {
            Ok(item) => {
                if let Some(data) = item.unwrap().unwrap().data {
                    let current_state = data.live_measurement.unwrap();
                    print_screen(current_state);
                    loop_counter = 0;
                } else {
                    return Err(Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "No valid data received.",
                    )));
                }
            }
            Err(_) => {
                loop_counter += 1;
                if loop_counter > config.reconnect_timeout {
                    return Ok(());
                }
            }
        }

        if check_user_shutdown(&receiver) == true {
            return Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "Shutdown requested",
            )));
        }
    }
}

async fn subscription_loop(
    config: AccessConfig,
    receiver: Receiver<bool>,
) -> Result<
    Option<Box<Subscription<StreamingOperation<LiveMeasurement>>>>,
    Box<dyn std::error::Error>,
> {
    let mut subscription = Box::new(
        connect_live_measurement(&config)
            .await
            .expect("Connection to Tibber could not be established."),
    );

    write!(stdout(), "Waiting for data ...").unwrap();
    stdout().flush().unwrap();

    loop {
        let final_result = loop_for_data(&config, subscription.as_mut(), &receiver).await;
        match final_result {
            Ok(_) => {
                if let Err(error) = subscription.stop().await {
                    println!("{:?}", error);
                    std::process::exit(exitcode::PROTOCOL)
                };

                let number_of_seconds = rand::thread_rng().gen_range(1..=60);
                println!(
                    "\nConnection lost, waiting for {}s before reconnecting.",
                    number_of_seconds
                );

                for _ in 0..=number_of_seconds {
                    if check_user_shutdown(&receiver) == true {
                        return Ok(None);
                    }
                    std::thread::sleep(time::Duration::from_secs(1));
                }

                subscription = Box::new(
                    connect_live_measurement(&config)
                        .await
                        .expect("Connection to Tibber could not be established."),
                );
            }
            Err(error) => {
                println!("\n{:?}", error);
                if let Some(io_err) = error.downcast_ref::<io::Error>() {
                    if io_err.kind() == io::ErrorKind::InvalidData {
                        return Err(error);
                    }
                }
                break;
            }
        }
    }

    Ok(Some(subscription))
}
