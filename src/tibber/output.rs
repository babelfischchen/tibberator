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

/// `OutputType` is an enum that represents the different types of output.
/// It can be one of the following: `Full` or `Silent`.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub enum OutputType {
    Full,
    Silent,
}

/// `TaxStyle` is an enum that represents the different styles of tax.
/// It can be one of the following: `Price`, `Percent`, or `None`.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub enum TaxStyle {
    Price,
    Percent,
    None,
}

/// `OutputConfig` is a struct that represents the configuration for output.
/// It contains the following fields: `output_type` and `tax_style`.
#[derive(Debug, Serialize, Deserialize)]
pub struct OutputConfig {
    output_type: OutputType,
    tax_style: TaxStyle,
}

/// The `Default` implementation for `OutputConfig` provides a default instance of `OutputConfig` with `output_type` as `Full` and `tax_style` as `Price`.
impl Default for OutputConfig {
    fn default() -> Self {
        OutputConfig {
            output_type: OutputType::Full,
            tax_style: TaxStyle::Price,
        }
    }
}

impl OutputConfig {
    /// The `is_silent` method for `OutputConfig` checks if the `output_type` is `Silent`.
    pub fn is_silent(&self) -> bool {
        self.output_type == OutputType::Silent
    }

    /// The `new` method for `OutputConfig` provides a way to create a new instance of `OutputConfig` with a given `output_type` and `tax_style` as `None`.
    pub fn new(output_type: OutputType) -> Self {
        OutputConfig {
            output_type,
            tax_style: TaxStyle::None,
        }
    }

    /// The `get_tax_style` method for `OutputConfig` provides a way to get a reference to the `tax_style`.
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
            let percent = if price_info.total != 0. {
                (price_info.tax / price_info.total * 100.).clamp(0., 100.)
            } else {
                0.
            };
            format!(" Tax: {:.1} %", percent)
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
