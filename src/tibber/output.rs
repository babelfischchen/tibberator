#[cfg(test)]
#[path = "output_tests/mod.rs"]
mod output_tests;

use crate::tibber::data_handling::live_measurement;
use crate::tibber::data_types::PriceInfo;
use chrono::{DateTime, FixedOffset, Local, Timelike};
use crossterm::{
    cursor, execute, queue,
    style::{SetForegroundColor, Stylize},
    terminal::{Clear, ClearType},
};

use log::{error, warn};

use std::{
    borrow::Borrow,
    io::{stdout, Write},
};

use serde::{Deserialize, Serialize};

/// `OutputType` is an enum that represents the different types of output.
/// It can be one of the following: `Full` or `Silent`.
#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub enum OutputType {
    Full,
    Silent,
}

/// `TaxStyle` is an enum that represents the different styles of tax.
/// It can be one of the following: `Price`, `Percent`, or `None`.
#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub enum TaxStyle {
    Price,
    Percent,
    None,
}

/// `OutputConfig` is a struct that represents the configuration for output.
/// It contains the following fields: `output_type` and `tax_style`.
#[derive(Debug, Serialize, Deserialize, PartialEq, Clone, Hash, Eq)]
pub enum DisplayMode {
    Prices,
    PricesTomorrow,
    Consumption,
    Cost,
    CostLast30Days,
    CostLast12Months,
    AllYears,
}

impl DisplayMode {
    /// Returns the next display mode in the cycle.
    ///
    /// This method provides a way to cycle through the available display modes in a fixed order:
    /// Prices → PricesTomorrow → Consumption → Cost → CostLast30Days → CostLast12Months → AllYears → Prices
    ///
    /// # Returns
    ///
    /// The next `DisplayMode` variant in the cycle.
    ///
    pub fn next(&self) -> DisplayMode {
        match self {
            DisplayMode::Prices => DisplayMode::PricesTomorrow,
            DisplayMode::PricesTomorrow => DisplayMode::Consumption,
            DisplayMode::Consumption => DisplayMode::Cost,
            DisplayMode::Cost => DisplayMode::CostLast30Days,
            DisplayMode::CostLast30Days => DisplayMode::CostLast12Months,
            DisplayMode::CostLast12Months => DisplayMode::AllYears,
            DisplayMode::AllYears => DisplayMode::Prices,
        }
    }

    /// Returns the previous display mode in the cycle.
    ///
    /// This method provides a way to cycle through the available display modes in reverse order:
    /// Prices → AllYears → CostLast12Months → CostLast30Days → Cost → Consumption → PricesTomorrow → Prices
    ///
    /// # Returns
    ///
    /// The previous `DisplayMode` variant in the cycle.
    ///
    pub fn prev(&self) -> DisplayMode {
        match self {
            DisplayMode::Prices => DisplayMode::AllYears,
            DisplayMode::PricesTomorrow => DisplayMode::Prices,
            DisplayMode::Consumption => DisplayMode::PricesTomorrow,
            DisplayMode::Cost => DisplayMode::Consumption,
            DisplayMode::CostLast30Days => DisplayMode::Cost,
            DisplayMode::CostLast12Months => DisplayMode::CostLast30Days,
            DisplayMode::AllYears => DisplayMode::CostLast12Months,
        }
    }
}

/// `GuiMode` is an enum that represents the different modes of the GUI. It can be one of the following: `Simple` or `Advanced`.
#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub enum GuiMode {
    Simple,
    Advanced,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OutputConfig {
    output_type: OutputType,
    tax_style: TaxStyle,
    pub display_mode: DisplayMode,
    pub gui_mode: GuiMode,
}

/// The `Default` implementation for `OutputConfig` provides a default instance of `OutputConfig` with `output_type` as `Full`, `tax_style` as `Price`, and `display_mode` as `Prices`.
impl Default for OutputConfig {
    fn default() -> Self {
        OutputConfig {
            output_type: OutputType::Full,
            tax_style: TaxStyle::Price,
            display_mode: DisplayMode::Prices,
            gui_mode: GuiMode::Advanced,
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
            gui_mode: GuiMode::Advanced,
        }
    }

    /// Creates a new OutputConfig with the specified display mode
    ///
    /// This method allows for method chaining when configuring the OutputConfig,
    /// setting the display mode to the specified value.
    ///
    /// # Arguments
    ///
    /// * `display_mode` - The DisplayMode to set for this configuration
    ///
    /// # Returns
    ///
    /// The modified OutputConfig with the new display mode
    ///
    pub fn with_display_mode(mut self, display_mode: DisplayMode) -> Self {
        self.display_mode = display_mode;
        self
    }

    /// Creates a new OutputConfig with the specified gui mode
    ///
    /// This method allows for method chaining when configuring the OutputConfig,
    /// setting the gui mode to the specified value.
    ///
    /// # Arguments
    ///
    /// * `gui_mode` - The GuiMode to set for this configuration
    ///
    /// # Returns
    ///
    /// The modified OutputConfig with the new gui mode
    ///
    pub fn with_gui_mode(mut self, gui_mode: GuiMode) -> Self {
        self.gui_mode = gui_mode;
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
    bar_graph_data: &Option<(Vec<f64>, String, DateTime<FixedOffset>)>,
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
    if let Some((bar_data, bar_label, _)) = bar_graph_data {
        if let Err(e) = display_bar_graph(bar_data, bar_label, &mut std::io::stdout()) {
            error!("Error displaying bar graph: {}", e);
        }
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
pub fn display_bar_graph<W: Write>(
    data: &Vec<f64>,
    label: &str,
    writer: &mut W,
) -> Result<(), std::io::Error> {
    // Check if the data contains exactly 24 values
    if data.len() != 24 {
        warn!(
            "Warning: The data must contain exactly 24 values. Found {} values.",
            data.len()
        );
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "The data must contain exactly 24 values. Found {} values.",
                data.len()
            ),
        ));
    }

    let min_value = data.iter().cloned().fold(f64::INFINITY, f64::min);
    let max_value = data.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let bar_height = 15; // Height of the bar graph in rows

    writeln!(writer, "\n {}", label)?;

    // Handle the case where all values are the same
    let (effective_min, effective_max) = if min_value == max_value {
        // If all values are the same, create a small range around the value
        // to avoid division by zero
        (min_value - 0.5, max_value + 0.5)
    } else {
        (min_value, max_value)
    };

    // Draw a horizontal line at zero if we have both positive and negative values
    let has_negative = effective_min < 0.0;
    let has_positive = effective_max > 0.0;
    let zero_row = if has_negative && has_positive {
        // Calculate which row corresponds to zero
        let zero_percentage = (0.0 - effective_min) / (effective_max - effective_min);
        bar_height - (zero_percentage * bar_height as f64).round() as usize
    } else {
        // If all values are negative or all positive, no zero line needed
        bar_height + 1 // Out of visible range
    };

    // Calculate the y-axis labels
    let y_axis_labels = (0..bar_height)
        .map(|i| {
            let value = effective_min
                + (effective_max - effective_min) * i as f64 / (bar_height - 1) as f64;
            format!("{:8.3} ", value)
        })
        .collect::<Vec<_>>();

    // Get the current hour (0-23)
    let current_hour = Local::now().hour() as usize;

    // Print the bars vertically
    for row in 0..bar_height + 2 {
        if row < bar_height {
            // Print the y-axis label
            write!(writer, "{}", y_axis_labels[bar_height - row - 1])?;
        } else if row == bar_height {
            // Print the x-axis label header
            write!(writer, "        ")?;
        } else {
            // Print the hour labels at the bottom
            write!(writer, "        ")?;
        }

        for (index, &value) in data.iter().enumerate() {
            if row == bar_height + 1 {
                // Print the hour labels at the bottom
                if index == current_hour {
                    write!(writer, " \x1b[31m{:02}\x1b[0m", index)?; // Red color for the current hour
                } else {
                    write!(writer, " {:02}", index)?;
                }
            } else if row == zero_row && has_negative && has_positive {
                // Draw the zero line if we have both positive and negative values
                if index == current_hour {
                    write!(writer, "\x1b[31m─\x1b[0m  ")?; // Red color for the current hour
                } else {
                    write!(writer, "─  ")?;
                }
            } else if row < bar_height {
                // Calculate the normalized value to determine bar height
                let normalized_percentage =
                    (value - effective_min) / (effective_max - effective_min);
                let bar_length =
                    ((normalized_percentage * bar_height as f64).round() as usize).max(1);

                // For positive values, draw the bar above the zero line
                if value >= 0.0 && zero_row >= row && bar_length >= (bar_height - row) {
                    if index == current_hour {
                        write!(writer, "\x1b[31m█\x1b[0m  ")?; // Red color for the current hour
                    } else {
                        write!(writer, "█  ")?;
                    }
                }
                // For negative values, draw the bar below the zero line
                else if value < 0.0 && zero_row <= row && bar_length >= row {
                    if index == current_hour {
                        write!(writer, "\x1b[31m█\x1b[0m  ")?; // Red color for the current hour
                    } else {
                        write!(writer, "█  ")?;
                    }
                } else {
                    write!(writer, "   ")?;
                }
            } else {
                write!(writer, "   ")?;
            }
        }
        writeln!(writer)?;
    }

    writer.flush()?;
    Ok(())
}
