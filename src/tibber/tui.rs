use std::{collections::HashMap, io, time::Duration};

use chrono::{DateTime, Datelike, FixedOffset, Local, Timelike};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::{cursor, execute};
use ratatui::prelude::*;
use ratatui::text::Line;
use ratatui::widgets::*;

use crate::tibber::cache_expired;
use crate::tibber::data_handling::live_measurement;
use crate::tibber::data_types::PriceInfo;
use crate::tibber::output::DisplayMode;

/// Represents the application state
#[derive(Debug)]
pub struct AppState {
    /// Whether the application should exit
    pub should_quit: bool,
    /// Current live measurement data
    pub measurement: Option<live_measurement::LiveMeasurementLiveMeasurement>,
    /// Current price information
    pub price_info: Option<PriceInfo>,
    /// Estimated daily fees
    pub estimated_daily_fees: Option<f64>,
    /// Cached bar graph data (values, label, and timestamp)
    pub cached_bar_graph: HashMap<DisplayMode, (Vec<f64>, String, DateTime<FixedOffset>)>,
    /// Display mode (prices or consumption)
    pub display_mode: DisplayMode,
    /// Status message
    pub status: String,
    /// Flag indicating if data needs to be refreshed
    pub data_needs_refresh: bool,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            should_quit: false,
            measurement: None,
            price_info: None,
            estimated_daily_fees: None,
            cached_bar_graph: HashMap::new(),
            display_mode: DisplayMode::Prices,
            status: String::from("Waiting for data..."),
            data_needs_refresh: false,
        }
    }
}

enum TimeInterval {
    Hourly,
    HourlyNoHighlight,
    Last30Days,
    Last12Months,
    Years,
}

/// Initialize the terminal for TUI rendering
///
/// This function prepares the terminal for rendering the TUI by enabling raw mode,
/// entering an alternate screen, and hiding the cursor. It returns a Terminal instance
/// that can be used for rendering.
///
/// # Returns
///
/// A `Result` containing either:
/// - `Ok(Terminal<CrosstermBackend<io::Stdout>>)` - The initialized terminal instance
/// - `Err(io::Error)` - An error if terminal initialization fails
///
pub fn init_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>, io::Error> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, cursor::Hide)?;
    Terminal::new(CrosstermBackend::new(stdout))
}

/// Restore the terminal to its original state
///
/// This function restores the terminal to its original state after TUI rendering
/// by disabling raw mode, leaving the alternate screen, showing the cursor, and
/// ensuring the cursor is visible.
///
/// # Arguments
///
/// * `terminal` - A mutable reference to the terminal instance to restore
///
/// # Returns
///
/// A `Result` indicating success or an IO error
///
pub fn restore_terminal(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
) -> Result<(), io::Error> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, cursor::Show)?;
    terminal.show_cursor()?;
    Ok(())
}

/// Handle keyboard events
///
/// This function polls for keyboard events and updates the application state
/// based on user input. It handles quitting the application (q key), cycling
/// through display modes (d and s keys), and other keyboard interactions.
///
/// # Arguments
///
/// * `app_state` - A mutable reference to the application state to update
///
/// # Returns
///
/// A `Result` indicating success or an IO error
///
pub fn handle_events(app_state: &mut AppState) -> Result<(), io::Error> {
    if event::poll(Duration::from_millis(100))? {
        if let Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Press {
                match key.code {
                    KeyCode::Char('q') => {
                        app_state.should_quit = true;
                    }
                    KeyCode::Char('d') => {
                        app_state.display_mode = app_state.display_mode.next();

                        // Check if the cache for the current display mode is expired
                        let cache_expired = cache_expired(&app_state);

                        app_state.data_needs_refresh =
                            cache_expired || app_state.data_needs_refresh;
                    }
                    KeyCode::Char('s') => {
                        app_state.display_mode = app_state.display_mode.prev();

                        // Check if the cache for the current display mode is expired
                        let cache_expired = cache_expired(&app_state);

                        app_state.data_needs_refresh =
                            cache_expired || app_state.data_needs_refresh;
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

/// Draw the UI
///
/// This function renders the complete user interface by drawing all UI components
/// including the header, main content, bar graph, and footer. It uses a layout
/// system to organize the different sections of the interface.
///
/// # Arguments
///
/// * `frame` - A mutable reference to the frame where UI elements will be rendered
/// * `app_state` - A reference to the current application state
///
pub fn draw_ui(frame: &mut Frame, app_state: &AppState) {
    // Create the layout
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Length(6), // Main content
            Constraint::Min(20),   // Bar graph
            Constraint::Length(3), // Footer
        ])
        .split(frame.area());

    // Draw the header
    draw_header(frame, app_state, chunks[0]);

    // Draw the main content
    draw_main_content(frame, app_state, chunks[1]);

    // Draw the bar graph
    draw_bar_graph(frame, app_state, chunks[2]);

    // Draw the footer
    draw_footer(frame, chunks[3]);
}

/// Draw the header section
fn draw_header(frame: &mut Frame, app_state: &AppState, area: Rect) {
    let header_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    draw_title_block(frame, app_state, header_chunks[0]);
    draw_time_block(frame, app_state, header_chunks[1]);
}

/// Draw the main title block
fn draw_title_block(frame: &mut Frame, app_state: &AppState, area: Rect) {
    // Create a block with borders and title
    let block = Block::default()
        .title("Tibber Dashboard")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::from_u32(0x0023B8CC)));

    // Create the inner area within the block
    let inner_area = block.inner(area);

    // Render the block itself
    frame.render_widget(block, area);

    // Create a layout for the content inside the block
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(0)
        .constraints([Constraint::Min(1)].as_ref())
        .split(inner_area);

    // Create and render the status text inside the block
    let status_text = Paragraph::new(app_state.status.to_string())
        .style(Style::default().fg(Color::from_u32(0x0023B8CC)))
        .alignment(Alignment::Center);

    // Render the status text in the inner area
    frame.render_widget(status_text, chunks[0]);
}

fn draw_time_block(frame: &mut Frame, app_state: &AppState, area: Rect) {
    // Get current time for comparison
    let now = Local::now();

    // Get and format the timestamp
    let (time, is_old) = app_state.measurement.as_ref().map_or(
        (now.format("%H:%M:%S").to_string(), false),
        |data| {
            let timestamp = DateTime::parse_from_str(data.timestamp.as_str(), "%+").unwrap();
            let time_str = timestamp.format("%H:%M:%S").to_string();

            // Calculate if timestamp is more than a minute old
            let duration = now.signed_duration_since(timestamp.with_timezone(&Local));
            let is_old = duration.num_seconds() > 60;

            (time_str, is_old)
        },
    );

    // Set text color based on timestamp age
    let color = if is_old {
        Color::Red
    } else {
        Color::White // or whatever your default color is
    };

    let time_paragraph = Paragraph::new(time)
        .block(Block::default().borders(Borders::ALL))
        .style(Style::default().fg(color))
        .alignment(Alignment::Center);

    frame.render_widget(time_paragraph, area);
}

/// Draw the main content section
fn draw_main_content(frame: &mut Frame, app_state: &AppState, area: Rect) {
    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    // Left side - Power information
    let power_block = Block::default().title("Power").borders(Borders::ALL);

    let mut power_text = vec![Line::from("Waiting for data...")];

    if let Some(measurement) = &app_state.measurement {
        let power_production = measurement.power_production.unwrap_or(0.0);

        if power_production > 0.0 {
            power_text = vec![Line::from(format!(
                "Current Power production: {:.1} W",
                power_production
            ))
            .style(Style::default().fg(Color::Green))];
        } else {
            power_text = vec![Line::from(format!(
                "Current Power consumption: {:.1} W",
                measurement.power
            ))
            .style(Style::default().fg(Color::Red))];
        }

        // Add consumption/production today
        power_text.push(Line::from(""));
        power_text.push(Line::from(format!(
            "Consumption today: {:.3} kWh",
            measurement.accumulated_consumption
        )));
        power_text.push(Line::from(format!(
            "Production today: {:.3} kWh",
            measurement.accumulated_production
        )));
    }

    let power_paragraph = Paragraph::new(power_text)
        .block(power_block)
        .wrap(Wrap { trim: true });

    frame.render_widget(power_paragraph, main_chunks[0]);

    // Right side - Price information
    let price_block = Block::default().title("Price").borders(Borders::ALL);

    let mut price_text = vec![Line::from("Waiting for price data...")];

    if let Some(price_info) = &app_state.price_info {
        let price_color = match price_info.level {
            crate::tibber::data_handling::PriceLevel::VeryCheap => Color::LightGreen,
            crate::tibber::data_handling::PriceLevel::Cheap => Color::Green,
            crate::tibber::data_handling::PriceLevel::Normal => Color::Yellow,
            crate::tibber::data_handling::PriceLevel::Expensive => Color::LightRed,
            crate::tibber::data_handling::PriceLevel::VeryExpensive => Color::Red,
            _ => Color::White,
        };

        price_text = vec![
            Line::from(format!(
                "Current price: {:.3} {}/kWh",
                price_info.total, price_info.currency
            ))
            .style(Style::default().fg(price_color)),
            Line::from(format!(
                "Tax: {:.3} {}/kWh",
                price_info.tax, price_info.currency
            )),
        ];

        // Add cost today if available
        if let Some(measurement) = &app_state.measurement {
            if let Some(cost) = measurement.accumulated_cost {
                if let Some(currency) = &measurement.currency {
                    price_text.push(Line::from(""));
                    price_text.push(Line::from(format!(
                        "Cost today: {:.2} {}",
                        cost + app_state.estimated_daily_fees.unwrap_or(0.0),
                        currency
                    )));
                }
            }
        }
    }

    let price_paragraph = Paragraph::new(price_text)
        .block(price_block)
        .wrap(Wrap { trim: true });

    frame.render_widget(price_paragraph, main_chunks[1]);
}

fn create_month_string(month: usize) -> String {
    // create a month string (Jan, Feb, ...) from the month number
    let months = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    if month < 1 || month > 12 {
        return String::from("Invalid month");
    }
    return String::from(months[month - 1]);
}

fn create_bar_data(data: &Vec<f64>, interval: TimeInterval) -> Vec<Bar<'_>> {
    let right_now = Local::now();
    let current_hour = right_now.hour() as usize;
    let current_date = right_now.naive_local();

    let min_value = data.iter().cloned().fold(f64::INFINITY, f64::min);
    let max_value = data.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

    let (effective_min, effective_max) = if min_value == max_value {
        (min_value - 0.5, max_value + 0.5)
    } else {
        (min_value, max_value)
    };

    let bar_style = Style::default().fg(Color::from_u32(0x0023B8CC));
    let highlight_style = Style::default().fg(Color::from_u32(0xF55249));

    data.iter()
        .enumerate()
        .map(|(index, &value)| {
            let scaled_value = (((value - effective_min) / (effective_max - effective_min) * 90.0)
                + 10.0)
                .clamp(10.0, 100.0);

            let label_text = match interval {
                TimeInterval::Hourly | TimeInterval::HourlyNoHighlight => format!("{:02}", index),
                TimeInterval::Last30Days => {
                    let date = current_date - chrono::Duration::days(30 - index as i64);
                    date.format("%d").to_string()
                }
                TimeInterval::Last12Months => {
                    // For Last12Months: index 0 = 11 months ago, index 11 = current month
                    let months_ago = 12 - index as i32;

                    // Calculate the date by going back months_ago months
                    let mut month = current_date.month() as i32 - months_ago;

                    // Adjust month if we went to previous year
                    while month <= 0 {
                        month += 12;
                    }

                    // Format only the month number
                    create_month_string((month) as usize)
                }
                TimeInterval::Years => {
                    let year = current_date.year() - data.len() as i32 + index as i32 + 1;
                    format!("{}", year)
                }
            };

            let is_highlighted = match interval {
                TimeInterval::Hourly => index == current_hour,
                TimeInterval::HourlyNoHighlight => index == 24, // one beyond last entry, no highlight
                TimeInterval::Last30Days => index == 30, // one beyond last entry, no highlight
                TimeInterval::Last12Months => index == 12, // Highlight the current month
                TimeInterval::Years => index == data.len() - 1, // Highlight the current year
            };

            let value_style = if is_highlighted {
                highlight_style
            } else {
                bar_style
            };
            let bar_color = value_style.fg.unwrap_or(Color::Blue);

            let label_style = if is_highlighted {
                highlight_style
            } else {
                Style::default()
            };

            Bar::default()
                .value(scaled_value as u64)
                .label(Line::from(Span::styled(label_text, label_style)))
                .text_value(format!("{:.2}", value))
                .style(value_style)
                .value_style(Style::default().fg(Color::White).bg(bar_color))
        })
        .collect()
}

fn get_time_interval(display_mode: &DisplayMode) -> TimeInterval {
    match display_mode {
        DisplayMode::Prices => TimeInterval::Hourly,
        DisplayMode::PricesTomorrow => TimeInterval::HourlyNoHighlight,
        DisplayMode::Consumption => TimeInterval::Hourly,
        DisplayMode::Cost => TimeInterval::Hourly,
        DisplayMode::CostLast30Days => TimeInterval::Last30Days,
        DisplayMode::CostLast12Months => TimeInterval::Last12Months,
        DisplayMode::AllYears => TimeInterval::Years,
    }
}

/// Draw the bar graph section
fn draw_bar_graph(frame: &mut Frame, app_state: &AppState, area: Rect) {
    if let Some((data, label, _)) = app_state.cached_bar_graph.get(&app_state.display_mode) {
        let bar_block = Block::default().title(label.clone()).borders(Borders::ALL);
        let inner_area = bar_block.inner(area);

        let time_interval = get_time_interval(&app_state.display_mode);
        let max_bar_width = match time_interval {
            TimeInterval::Years => 16,
            TimeInterval::Last12Months => 11,
            TimeInterval::Last30Days => 4,
            TimeInterval::Hourly => 6,
            TimeInterval::HourlyNoHighlight => 6,
        };

        // Calculate the optimal bar width to fit in the available space
        let bar_data: Vec<Bar> = create_bar_data(data, time_interval);
        let num_bars = bar_data.len();
        let bar_gap = 1;
        let available_width = inner_area.width as usize;

        // Calculate optimal bar width
        // For n bars, we need space for n bars and (n-1) gaps
        // So: n*bar_width + (n-1)*bar_gap <= available_width
        // Solving for bar_width: bar_width <= (available_width - (n-1)*bar_gap) / n
        let optimal_bar_width = if num_bars > 0 {
            let width = (available_width - ((num_bars - 1) * bar_gap)) / num_bars;
            width.min(max_bar_width).max(1) as u16
        } else {
            max_bar_width as u16
        };

        // Create bar data in format (&str, u64)
        let chart = BarChart::default()
            .bar_width(optimal_bar_width)
            .bar_gap(bar_gap as u16)
            .data(BarGroup::default().bars(&bar_data));

        frame.render_widget(bar_block, area);

        // Total chart width including some padding for better readability
        let total_width = data.len() * (optimal_bar_width as usize + bar_gap) as usize + 2;

        // Create a centered layout for the chart if it's smaller than the available space
        if total_width < available_width {
            let centered_layout = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length((inner_area.width - total_width as u16) / 2),
                    Constraint::Length(total_width as u16),
                    Constraint::Min(0),
                ])
                .split(inner_area);

            // Render the chart in the centered area
            frame.render_widget(chart, centered_layout[1]);
        } else {
            // If the chart is wider than the available space, render it normally
            frame.render_widget(chart, inner_area);
        }
    } else {
        // If no data is available, show a message
        let block = Block::default()
            .title("Fetching data. Please wait...")
            .borders(Borders::ALL);

        frame.render_widget(block, area);
    }
}

/// Draw the footer section
fn draw_footer(frame: &mut Frame, area: Rect) {
    let footer_text = Line::from(vec![
        Span::styled("[q] ", Style::default().fg(Color::Red)),
        Span::styled("Quit, ", Style::default().fg(Color::White)),
        Span::styled("[d] ", Style::default().fg(Color::Green)),
        Span::styled("Next Display, ", Style::default().fg(Color::White)),
        Span::styled("[s] ", Style::default().fg(Color::Yellow)),
        Span::styled("Previous Display", Style::default().fg(Color::White)),
    ]);

    let footer_block = Block::default()
        .title("Shortcuts")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Gray));

    let footer_paragraph = Paragraph::new(footer_text)
        .block(footer_block)
        .style(Style::default().fg(Color::White))
        .alignment(Alignment::Center);

    frame.render_widget(footer_paragraph, area);
}
