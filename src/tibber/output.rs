use crate::tibber::{live_measurement, PriceInfo};
use chrono::{DateTime, Local, Timelike};
use crossterm::{
    cursor, execute, queue,
    style::{SetForegroundColor, Stylize},
    terminal::{Clear, ClearType},
};

use log::warn;

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
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub enum DisplayMode {
    Prices,
    Consumption,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OutputConfig {
    output_type: OutputType,
    tax_style: TaxStyle,
    pub display_mode: DisplayMode,
}

/// The `Default` implementation for `OutputConfig` provides a default instance of `OutputConfig` with `output_type` as `Full`, `tax_style` as `Price`, and `display_mode` as `Prices`.
impl Default for OutputConfig {
    fn default() -> Self {
        OutputConfig {
            output_type: OutputType::Full,
            tax_style: TaxStyle::Price,
            display_mode: DisplayMode::Prices,
        }
    }
}

impl OutputConfig {
    /// The `is_silent` method for `OutputConfig` checks if the `output_type` is `Silent`.
    pub fn is_silent(&self) -> bool {
        self.output_type == OutputType::Silent
    }

    /// The `new` method for `OutputConfig` provides a way to create a new instance of `OutputConfig` with a given `output_type`, `tax_style` as `None`, and `display_mode` as `Prices`.
    pub fn new(output_type: OutputType) -> Self {
        OutputConfig {
            output_type,
            tax_style: TaxStyle::None,
            display_mode: DisplayMode::Prices,
        }
    }

    /// Creates a new OutputConfig with the specified display mode
    pub fn with_display_mode(mut self, display_mode: DisplayMode) -> Self {
        self.display_mode = display_mode;
        self
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
    bar_graph_data: &Option<(Vec<f64>, &str)>,
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

    // display a bar graph if `bar_graph_data` contains Some value
    if let Some((bar_data, bar_label)) = bar_graph_data {
        display_bar_graph(bar_data, bar_label);
    }
}

/// # `display_bar_graph`
///
/// Displays a bar graph based on the provided hourly data.
///
/// ## Parameters
/// - `data`: A vector of f64 values representing hourly data.
/// - `label`: A string label for the data type (e.g., "Price per kWh" or "Consumption").
///
/// ## Behavior
/// - Clears the terminal screen.
/// - Displays a bar graph with 24 bars, one for each hourly period.
/// - Labels the graph with the provided label.
/// - Logs a warning and quits if the number of data points is not 24.
///
pub fn display_bar_graph(data: &Vec<f64>, label: &str) {
    // Check if the data contains exactly 24 values
    if data.len() != 24 {
        warn!(
            "Warning: The data must contain exactly 24 values. Found {} values.",
            data.len()
        );
        return;
    }

    let min_value = data.iter().cloned().fold(f64::INFINITY, f64::min);
    let max_value = data.iter().cloned().fold(0.0, f64::max);
    let bar_height = 15; // Height of the bar graph in rows

    writeln!(stdout(), "\n {}", label).unwrap();

    // Calculate the y-axis labels
    let y_axis_labels = (0..=bar_height)
        .map(|i| {
            let value = min_value + (max_value - min_value) * i as f64 / bar_height as f64;
            format!("{:8.3} ", value)
        })
        .collect::<Vec<_>>();

    // Get the current hour (0-23)
    let current_hour = Local::now().hour() as usize;

    // Print the bars vertically
    for row in 0..bar_height + 2 {
        if row < bar_height {
            // Print the y-axis label
            write!(stdout(), "{}", y_axis_labels[bar_height - row]).unwrap();
        } else if row == bar_height {
            // Print the x-axis label header
            write!(stdout(), "        ").unwrap();
        } else {
            // Print the hour labels at the bottom
            write!(stdout(), "        ").unwrap();
        }

        for (index, &value) in data.iter().enumerate() {
            let bar_length = if max_value == min_value {
                1 // Ensure each bar has at least one unit of height if all values are the same
            } else {
                let length =
                    ((value - min_value) / (max_value - min_value) * bar_height as f64) as usize;
                length.max(1) // Ensure each bar has at least one unit of height
            };
            if row == bar_height + 1 {
                // Print the hour labels at the bottom
                if index == current_hour {
                    write!(stdout(), " \x1b[31m{:02}\x1b[0m", index).unwrap(); // Red color for the current hour
                } else {
                    write!(stdout(), " {:02}", index).unwrap();
                }
            } else if row < bar_height && bar_length >= (bar_height - row) {
                if index == current_hour {
                    write!(stdout(), "\x1b[31m█\x1b[0m  ").unwrap(); // Red color for the current hour
                } else {
                    write!(stdout(), "█  ").unwrap();
                }
            } else {
                write!(stdout(), "   ").unwrap();
            }
        }
        writeln!(stdout()).unwrap();
    }

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
