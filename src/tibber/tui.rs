use std::io;
use std::time::Duration;

use chrono::{Local, Timelike};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::{cursor, execute};
use ratatui::prelude::*;
use ratatui::text::Line;
use ratatui::widgets::*;

use crate::tibber::output::DisplayMode;
use crate::tibber::{live_measurement, PriceInfo};

/// Represents the application state
pub struct AppState {
    /// Whether the application should exit
    pub should_quit: bool,
    /// Current live measurement data
    pub measurement: Option<live_measurement::LiveMeasurementLiveMeasurement>,
    /// Current price information
    pub price_info: Option<PriceInfo>,
    /// Bar graph data (values and label)
    pub bar_graph_data: Option<(Vec<f64>, String)>,
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
            bar_graph_data: None,
            display_mode: DisplayMode::Prices,
            status: String::from("Waiting for data..."),
            data_needs_refresh: false,
        }
    }
}

/// Initialize the terminal for TUI rendering
pub fn init_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>, io::Error> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, cursor::Hide)?;
    Terminal::new(CrosstermBackend::new(stdout))
}

/// Restore the terminal to its original state
pub fn restore_terminal(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
) -> Result<(), io::Error> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, cursor::Show)?;
    terminal.show_cursor()?;
    Ok(())
}

/// Handle keyboard events
pub fn handle_events(app_state: &mut AppState) -> Result<(), io::Error> {
    if event::poll(Duration::from_millis(100))? {
        if let Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Press {
                match key.code {
                    KeyCode::Char('q') => {
                        app_state.should_quit = true;
                    }
                    KeyCode::Char('d') => {
                        app_state.display_mode = match app_state.display_mode {
                            DisplayMode::Prices => DisplayMode::Consumption,
                            DisplayMode::Consumption => DisplayMode::Prices,
                        };
                        app_state.data_needs_refresh = true;
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

/// Draw the UI
pub fn draw_ui(frame: &mut Frame, app_state: &AppState) {
    // Create the layout
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // Header
            Constraint::Length(6),    // Main content
            Constraint::Min(20), // Bar graph
        ])
        .split(frame.area());

    // Draw the header
    draw_header(frame, chunks[0]);

    // Draw the main content
    draw_main_content(frame, app_state, chunks[1]);

    // Draw the bar graph
    draw_bar_graph(frame, app_state, chunks[2]);
}

/// Draw the header section
fn draw_header(frame: &mut Frame, area: Rect) {
    let header_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    // Title and time
    let title = "Tibberator";
    let time = Local::now().format("%H:%M:%S").to_string();

    let title_block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::from_u32(0x0023B8CC)));

    let time_paragraph = Paragraph::new(time)
        .block(Block::default().borders(Borders::ALL))
        .alignment(Alignment::Center);

    frame.render_widget(title_block, header_chunks[0]);
    frame.render_widget(time_paragraph, header_chunks[1]);
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
                    price_text.push(Line::from(format!("Cost today: {:.2} {}", cost, currency)));
                }
            }
        }
    }

    let price_paragraph = Paragraph::new(price_text)
        .block(price_block)
        .wrap(Wrap { trim: true });

    frame.render_widget(price_paragraph, main_chunks[1]);
}

/// Draw the bar graph section
fn draw_bar_graph(frame: &mut Frame, app_state: &AppState, area: Rect) {
    if let Some((data, label)) = &app_state.bar_graph_data {
        let current_hour = Local::now().hour() as usize;

        // Find min and max values for scaling
        let min_value = data.iter().cloned().fold(f64::INFINITY, f64::min);
        let max_value = data.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

        // Handle the case where all values are the same
        let (effective_min, effective_max) = if min_value == max_value {
            (min_value - 0.5, max_value + 0.5)
        } else {
            (min_value, max_value)
        };

        let bar_block = Block::default().title(label.clone()).borders(Borders::ALL);

        // Set default bar style
        let bar_style = Style::default().fg(Color::from_u32(0x0023B8CC));
        let highlight_style = Style::default().fg(Color::from_u32(0xF55249));

        // Create bar data in format (&str, u64)
        let bar_data: Vec<Bar> = data
            .iter()
            .enumerate()
            .map(|(hour, &value)| {
                // Normalize value to 0-100 range for bar height
                let scaled_value =
                    (((value - effective_min) / (effective_max - effective_min) * 90.0) + 10.0)
                        .clamp(10.0, 100.0);

                let value_style = if hour == current_hour {
                    highlight_style
                } else {
                    bar_style
                };
                let bar_color = value_style.fg.unwrap_or(Color::Blue);

                let hour_style = if hour == current_hour {
                    highlight_style
                } else {
                    Style::default()
                };

                Bar::default()
                    .value(scaled_value as u64)
                    .label(Line::from(Span::styled(format!("{hour:02}"), hour_style)))
                    .text_value(format!("{value:.2}"))
                    .style(value_style)
                    .value_style(Style::default().fg(Color::White).bg(bar_color))
            })
            .collect();
        let chart = BarChart::default()
            .block(bar_block)
            .bar_width(4)
            .bar_gap(1)
            .data(BarGroup::default().bars(&bar_data));

        frame.render_widget(chart, area);
    } else {
        // If no data is available, show a message
        let block = Block::default()
            .title("No data available")
            .borders(Borders::ALL);

        frame.render_widget(block, area);
    }
}
