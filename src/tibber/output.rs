use crate::tibber::{live_measurement, PriceInfo};
use chrono::DateTime;
use crossterm::{
    cursor, execute, queue,
    style::{SetForegroundColor, Stylize},
    terminal::{Clear, ClearType},
};

use std::{
    borrow::Borrow,
    io::{stdout, Write},
};

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub enum OutputType {
    Full,
    Silent,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub enum TaxStyle {
    Price,
    Percent,
    None,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OutputConfig {
    output_type: OutputType,
    tax_style: TaxStyle,
}

impl Default for OutputConfig {
    fn default() -> Self {
        OutputConfig {
            output_type: OutputType::Full,
            tax_style: TaxStyle::Price,
        }
    }
}

impl OutputConfig {
    pub fn is_silent(&self) -> bool {
        self.output_type == OutputType::Silent
    }

    pub fn new(output_type: OutputType) -> Self {
        OutputConfig {
            output_type,
            tax_style: TaxStyle::None,
        }
    }

    pub fn get_tax_style(&self) -> &TaxStyle {
        self.tax_style.borrow()
    }
}

/// # `print_screen`
///
/// Prints a formatted screen display based on the provided `LiveMeasurement` data.
///
/// ## Parameters
/// - `tax_style`: The format of the tax output.
/// - `data`: A `LiveMeasurementLiveMeasurement` struct containing relevant measurement data.
/// - `price_info`: Data about the current price level
///
/// ## Behavior
/// - Clears the terminal screen.
/// - Displays information related to time, power consumption/production, cost, and energy consumption for today.
///
pub fn print_screen(
    tax_style: &TaxStyle,
    data: live_measurement::LiveMeasurementLiveMeasurement,
    price_info: &PriceInfo,
) {
    let tax_string = match tax_style {
        TaxStyle::Price => {
            format!(" Tax: {:.3} {}/kWh", price_info.tax, price_info.currency)
        }
        TaxStyle::Percent => {
            format!(" Tax: {:.1} %", price_info.tax / price_info.total * 100.)
        }
        TaxStyle::None => {
            return;
        }
    };

    let timestamp = DateTime::parse_from_str(&data.timestamp, "%+").unwrap();
    let str_timestamp = timestamp.format("%H:%M:%S");

    let mut line_number = 1;

    queue!(
        stdout(),
        Clear(ClearType::All),
        cursor::MoveTo(1, line_number)
    )
    .unwrap();

    let mut move_cursor = |row_increase: u16, column: Option<u16>| {
        line_number += row_increase;
        queue!(stdout(), cursor::MoveTo(column.unwrap_or(1), line_number)).unwrap();
    };

    let power_production = data.power_production.unwrap_or(0.);

    // time
    write!(stdout(), "Time:").unwrap();
    move_cursor(1, None);
    write!(stdout(), "{}", str_timestamp).unwrap();
    move_cursor(2, None);

    // current power
    if power_production == 0. {
        write!(stdout(), "{}", "Current Power consumption:".red()).unwrap();
        move_cursor(1, None);
        write!(stdout(), "{:.1} W", data.power).unwrap();
    }
    // current production
    else {
        write!(stdout(), "{}", "Current Power production:".green()).unwrap();
        move_cursor(1, None);
        write!(stdout(), "{:.1} W", power_production).unwrap();
    }

    // current price
    move_cursor(0, Some(10));
    execute!(
        stdout(),
        crossterm::style::Print("("),
        SetForegroundColor(
            price_info
                .level
                .to_color()
                .unwrap_or(crossterm::style::Color::White)
        ),
        crossterm::style::Print(format!(
            "{:.3} {}/kWh",
            price_info.total, price_info.currency
        )),
        crossterm::style::ResetColor,
        crossterm::style::Print(format!("{})", tax_string))
    )
    .unwrap();

    // cost today
    move_cursor(2, None);
    write!(stdout(), "Cost today:").unwrap();
    move_cursor(1, None);
    write!(
        stdout(),
        "{:.2} {}",
        data.accumulated_cost.unwrap_or(-1.),
        data.currency.unwrap_or(String::from("None"))
    )
    .unwrap();

    // consumption today
    move_cursor(2, None);
    write!(stdout(), "Consumption today:").unwrap();
    move_cursor(1, None);
    write!(stdout(), "{:.3} kWh", data.accumulated_consumption).unwrap();

    // production today
    move_cursor(2, None);
    write!(stdout(), "Production today:").unwrap();
    move_cursor(1, None);
    write!(stdout(), "{:.3} kWh", data.accumulated_production).unwrap();
    execute!(stdout(), cursor::Hide).unwrap();
    move_cursor(1, None);

    stdout().flush().unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_silent() {
        let mut config = OutputConfig::default();
        assert!(!config.is_silent());
        config.output_type = OutputType::Silent;
        assert!(config.is_silent());
    }
}
