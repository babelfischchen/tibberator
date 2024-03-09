use std::{
    io::{stdin, stdout, Read, Write},
    sync::mpsc::{self, Receiver},
};

use clap_v3::{App, Arg, ArgMatches};

use confy;

use crossterm::{
    cursor, execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen},
};
use ctrlc::set_handler;
use exitcode;
use futures::executor::block_on;
use graphql_ws_client::{graphql::StreamingOperation, Subscription};
use rand::Rng;
use tokio::time;

use tibberator::tibber::*;

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

async fn handle_reconnect(
    access_config: &AccessConfig,
    subscription: Box<Subscription<StreamingOperation<LiveMeasurement>>>,
    receiver: &Receiver<bool>,
) -> Result<Subscription<StreamingOperation<LiveMeasurement>>, LoopEndingError> {
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
            return Err(LoopEndingError::Shutdown);
        }
        std::thread::sleep(time::Duration::from_secs(1));
    }

    Ok(connect_live_measurement(&access_config).await)
}

async fn subscription_loop(
    config: Config,
    receiver: Receiver<bool>,
) -> Result<
    Option<Box<Subscription<StreamingOperation<LiveMeasurement>>>>,
    Box<dyn std::error::Error>,
> {
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
                        Ok(res) => {
                            subscription = Box::new(res)
                        },
                        Err(LoopEndingError::Shutdown) => {
                            return Ok(None);
                        },
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
