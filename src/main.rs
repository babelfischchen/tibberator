use std::{
    io::{stdout, Write},
    sync::mpsc::{self, Receiver, RecvTimeoutError},
    time::Duration,
};

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

use tibberator::tibber::{
    get_live_measurement, live_measurement::LiveMeasurementLiveMeasurement, AccessConfig,
    LiveMeasurement,
};

fn get_config() -> Result<AccessConfig, confy::ConfyError> {
    let config: AccessConfig = confy::load("Tibberator", "config")?;
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

    let mut subscription = block_on(get_live_measurement(&config));

    {
        match subscription {
            Ok(ref mut result) => {
                println!("Connection established");
                block_on(subscription_loop(result, receiver))
            }
            Err(error) => {
                println!("{:?}", error);
                std::process::exit(exitcode::PROTOCOL);
            }
        };
    }
    execute!(stdout(), LeaveAlternateScreen).unwrap();

    let stop_result = block_on(subscription.unwrap().stop());
    match stop_result {
        Ok(()) => std::process::exit(exitcode::OK),
        Err(error) => {
            println!("{:?}", error);
            std::process::exit(exitcode::PROTOCOL)
        }
    };
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

async fn subscription_loop(
    subscription: &mut Subscription<StreamingOperation<LiveMeasurement>>,
    receiver: Receiver<bool>,
) {
    write!(stdout(), "Waiting for data ...").unwrap();
    stdout().flush().unwrap();

    while let Some(item) = subscription.next().await {
        let current_state = item.unwrap().data.unwrap().live_measurement.unwrap();
        print_screen(current_state);

        let received_value = receiver.recv_timeout(Duration::from_millis(100));
        match received_value {
            Ok(value) => {
                if value == true {
                    break;
                }
            }
            Err(error) => match error {
                RecvTimeoutError::Timeout => {}
                RecvTimeoutError::Disconnected => {
                    println!("{:?}", error);
                    break;
                }
            },
        };
    }
}
