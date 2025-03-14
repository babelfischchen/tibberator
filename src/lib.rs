//! Module for easy access to the Tibber API
//!
//! This module contains various helper methods to connect to the Tibber API (see https://developer.tibber.com).
//! You need an access token in order to use the API.
pub mod tibber {
    use chrono::{DateTime, FixedOffset, Local, Timelike};
    use futures::{future, stream::StreamExt, task::Poll};
    use log::{debug, error, info, warn};
    use serde::{Deserialize, Serialize};
    use std::{
        cell::Cell,
        rc::Rc,
        sync::mpsc::{Receiver, RecvTimeoutError},
        sync::{Arc, Mutex},
        time::Instant,
    };

    pub use data_handling::{
        connect_live_measurement, estimate_daily_fees, fetch_home_data, get_home_ids,
        get_last_consumption_pages, get_todays_energy_consumption, get_todays_energy_prices,
        live_measurement, update_current_energy_price_info, AccessConfig, ConsumptionNode,
        LiveMeasurementOperation, LiveMeasurementSubscription, LoopEndingError, PriceInfo,
    };
    use output::{print_screen, DisplayMode, OutputConfig};
    use tui::AppState;

    use crate::html_logger::LogConfig;

    mod data_handling;
    pub mod output;
    pub mod tui;

    #[derive(Debug, Serialize, Deserialize)]
    pub struct Config {
        pub access: AccessConfig,
        pub output: OutputConfig,
        pub logging: LogConfig,
    }

    impl Default for Config {
        fn default() -> Self {
            Config {
                access: AccessConfig::default(),
                output: OutputConfig::default(),
                logging: LogConfig::default(),
            }
        }
    }
    /// # `check_user_shutdown`
    ///
    /// ## Description
    /// This function checks whether the user has requested a shutdown by monitoring a receiver channel.
    /// It waits for a short duration (1 second) to receive a value from the channel.
    /// If a value is received within the timeout, it returns the received value (indicating whether the user requested a shutdown).
    /// Otherwise, it handles the timeout or disconnection error and returns an appropriate boolean value.
    ///
    /// ## Parameters
    /// - `receiver`: A reference to a `Receiver<bool>` channel. This channel is used to receive signals related to user shutdown requests.
    ///
    /// ## Return Value
    /// - `true`: Indicates that the user requested a shutdown.
    /// - `false`: Indicates that no shutdown request was received within the timeout.
    ///
    /// ## Example
    /// ```rust
    ///   use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
    ///   use std::time::Duration;
    ///   use tibberator::tibber::check_user_shutdown;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    ///   let (sender, receiver) = channel();
    ///   assert!(check_user_shutdown(&receiver) == false);
    ///   sender.send(true).unwrap();
    ///   assert!(check_user_shutdown(&receiver) == true);
    /// # }
    /// ```
    pub fn check_user_shutdown(receiver: &Receiver<bool>) -> bool {
        let received_value = receiver.recv_timeout(std::time::Duration::from_secs(1));
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

    /// # `loop_for_data`
    ///
    /// Asynchronously processes streaming data from a subscription, monitoring for user shutdown requests
    /// and reconnect conditions. The function takes care of handling timeouts, disconnections, and invalid data.
    ///
    /// ## Parameters
    /// - `config`: A reference to the configuration settings.
    /// - `subscription`: A mutable reference to the data subscription.
    /// - `receiver`: A reference to a `Receiver<bool>` channel for monitoring user shutdown requests.
    ///
    /// ## Return Value
    /// - `Ok(())`: Indicates successful completion (shutdown requested).
    /// - `Err(LoopEndingError::Reconnect)`: Indicates the need to reconnect due to elapsed time.
    /// - `Err(LoopEndingError::InvalidData)`: Indicates no valid data received during the loop.
    ///
    /// ## Example
    /// ```rust
    ///   use tibberator::tibber::{
    ///                            tui::AppState,
    ///                            Config, loop_for_data,
    ///                            AccessConfig, connect_live_measurement,
    ///                           };
    ///   use std::sync::{Arc, Mutex};
    ///   use tokio::time;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    ///   let config = Config::default();
    ///   let mut subscription = connect_live_measurement(&config.access).await;
    ///   let app_state = Arc::new(Mutex::new(AppState::default()));
    ///
    ///   let state = app_state.clone();

    ///   tokio::spawn(async move {
    ///     std::thread::sleep(time::Duration::from_secs(3));
    ///     app_state.lock().unwrap().should_quit = true;
    ///   });
    ///   let result = loop_for_data(&config, &mut subscription, state).await;
    ///   assert!(result.is_ok());
    /// # }
    /// ```
    pub async fn loop_for_data(
        config: &Config,
        subscription: &mut LiveMeasurementSubscription,
        app_state: Arc<Mutex<AppState>>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let last_value_received = Rc::new(Cell::new(Instant::now()));
        let stop_fun = future::poll_fn(|_cx| {
            if app_state.lock().unwrap().should_quit {
                Poll::Ready(LoopEndingError::Shutdown)
            } else if last_value_received.get().elapsed().as_secs()
                > config.access.reconnect_timeout
            {
                Poll::Ready(LoopEndingError::Reconnect)
            } else {
                Poll::Pending
            }
        });

        if config.output.is_silent() {
            println!("\nOutput silent. Press CTRL+C to exit.");
        }

        let mut current_price_info = update_current_energy_price_info(&config.access, None).await?;

        // Fetch data based on display mode configuration
        let display_data =
            fetch_display_data(&config.access, &config.output.display_mode, &None).await?;

        let mut stream = subscription.take_until(stop_fun);
        loop {
            match tokio::time::timeout(
                std::time::Duration::from_secs(2 * config.access.reconnect_timeout),
                stream.by_ref().next(),
            )
            .await
            {
                Ok(Some(result)) => match result.unwrap().data {
                    Some(data) => {
                        let current_state = data.live_measurement.unwrap();
                        last_value_received.set(Instant::now());

                        debug!(target: "tibberator.mainloop", "Received power measurement: {} W", current_state.power);

                        if !config.output.is_silent() {
                            print_screen(
                                &config.output.get_tax_style(),
                                current_state,
                                &current_price_info,
                                &display_data,
                            );
                        }
                    }
                    None => {
                        error!(target: "tibberator.mainloop", "Invalid data received, shutting down.");
                        return Err(Box::new(LoopEndingError::InvalidData));
                    }
                },
                Ok(None) => break,
                Err(_) => {
                    info!(target: "tibberator.mainloop", "Connection timed out, reconecting...");
                    return Err(Box::new(LoopEndingError::Reconnect));
                }
            }

            current_price_info =
                update_current_energy_price_info(&config.access, Some(current_price_info)).await?;
        }
        match stream.take_result() {
            Some(LoopEndingError::Shutdown) => {
                info!(target: "tibberator.mainloop", "User shutdown requested.");
                Ok(())
            }
            Some(LoopEndingError::Reconnect) => Err(Box::new(LoopEndingError::Reconnect)),
            _ => Err(Box::new(LoopEndingError::InvalidData)),
        }
    }

    /// Checks if the cached bar graph data is expired based on the current display mode.
    ///
    /// This function examines the cache in `AppState` to determine whether the bar graph data
    /// associated with the current display mode has expired. It compares the cached timestamp
    /// with the current time and logs the expiration status for debugging purposes.
    ///
    /// # Arguments
    ///
    /// * `app_state` - A reference to the application state containing the cache and display mode.
    ///
    /// # Returns
    ///
    /// * `bool` - `true` if the cache is expired, otherwise `false`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tibberator::tibber::tui::AppState;
    /// use tibberator::tibber::cache_expired;

    /// let app_state = AppState::default(); // Assume this initializes with appropriate values
    /// let is_expired = cache_expired(&app_state);
    /// println!("Cache expired: {}", is_expired);
    /// ```
    pub fn cache_expired(app_state: &AppState) -> bool {
        app_state
            .cached_bar_graph
            .get(&app_state.display_mode)
            .map_or(true, |(_, _, timestamp)| {
                let now = Local::now().fixed_offset();
                let expired = now > *timestamp;
                debug!(
                    target: "tibberator.cache",
                    "Cache {} expired for mode: {:?} (current timestamp: {} vs. now {})",
                    if expired { "" } else { "not" },
                    app_state.display_mode,
                    *timestamp,
                    now
                );
                expired
            })
    }

    pub async fn fetch_display_data(
        access_config: &AccessConfig,
        display_mode: &DisplayMode,
        estimated_daily_fee: &Option<f64>,
    ) -> Result<Option<(Vec<f64>, String, DateTime<FixedOffset>)>, Box<dyn std::error::Error>> {
        match display_mode {
            output::DisplayMode::Prices => match get_todays_energy_prices(access_config).await {
                Ok(prices) => {
                    // Add one hour past the last starting time stamp so that the data expire at 12am the next day
                    let time = prices.last().ok_or(LoopEndingError::InvalidData)?.starts_at
                        + chrono::Duration::hours(1);
                    Ok(Some((
                        prices.into_iter().map(|p| p.total).collect(),
                        String::from("Energy Prices [EUR/kWh]"),
                        time,
                    )))
                }
                Err(error) => {
                    warn!(target: "tibberator.mainloop", "Failed to fetch today's energy prices: {:?}", error.to_string());
                    Ok(None)
                }
            },
            output::DisplayMode::Consumption => {
                match get_todays_energy_consumption(&access_config).await {
                    Ok(consumption) => {
                        let time = Local::now()
                            .with_minute(0)
                            .and_then(|t| t.with_second(0))
                            .and_then(|t| t.with_nanosecond(0))
                            .unwrap_or(Local::now())
                            .fixed_offset()
                            + chrono::Duration::hours(1);
                        Ok(Some((
                            consumption.into_iter().map(|c| c.consumption).collect(),
                            String::from("Energy Consumption [kWh]"),
                            time,
                        )))
                    }
                    Err(error) => {
                        warn!(target: "tibberator.mainloop", "Failed to fetch today's energy consumption: {:?}", error.to_string());
                        Ok(None)
                    }
                }
            }
            output::DisplayMode::Cost => {
                let current_hour = Local::now().hour() as usize;
                let (mut pages_so_far, last_data_time) = match get_last_consumption_pages(
                    access_config,
                    current_hour,
                )
                .await
                {
                    Ok((consumption_pages, last_data_time)) => (
                        consumption_pages
                            .into_iter()
                            .map(|c| c.total_cost + estimated_daily_fee.unwrap_or(0.0) / 24.0)
                            .collect(),
                        last_data_time + chrono::Duration::hours(1),
                    ),
                    Err(error) => {
                        warn!(target: "tibberator.mainloop", "Failed to fetch today's energy consumption: {:?}", error.to_string());
                        (Vec::new(), Local::now().fixed_offset())
                    }
                };

                if pages_so_far.is_empty() {
                    return Ok(None);
                }

                let description_string = String::from("Cost [EUR]");
                for _i in 0..(24 - current_hour) {
                    pages_so_far.push(0.0);
                }

                Ok(Some((pages_so_far, description_string, last_data_time)))
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use std::collections::HashMap;

        use super::*;
        use chrono::{DateTime, Duration, FixedOffset};
        use data_handling::connect_live_measurement;
        use output::OutputType;
        use serial_test::serial;
        use tokio::time::{timeout, Duration as TokioDuration};

        fn create_app_state() -> Arc<Mutex<AppState>> {
            Arc::new(Mutex::new(AppState {
                should_quit: false,
                measurement: None,
                price_info: None,
                estimated_daily_fees: None,
                cached_bar_graph: HashMap::new(),
                display_mode: DisplayMode::Prices,
                status: String::from("Waiting for data..."),
                data_needs_refresh: false,
            }))
        }

        #[tokio::test]
        async fn test_update_price_after_one_hour() {
            let config = AccessConfig::default();

            let price_info = PriceInfo::default();
            assert!(price_info.starts_at == DateTime::<FixedOffset>::default());
            let result_initial_update =
                update_current_energy_price_info(&config, Some(price_info)).await;
            assert!(result_initial_update.is_ok());
            let result_initial_update = result_initial_update.unwrap();
            assert_ne!(result_initial_update, PriceInfo::default());
            assert!(result_initial_update.starts_at > DateTime::<FixedOffset>::default());

            // time within one hour
            let price_info_within_an_hour = result_initial_update.clone();
            let updated_price_info_within_hour =
                update_current_energy_price_info(&config, Some(price_info_within_an_hour)).await;
            assert!(updated_price_info_within_hour.is_ok());
            assert_eq!(
                updated_price_info_within_hour.unwrap(),
                result_initial_update
            );

            // set time one hour lower
            let mut price_info_one_hour_before = result_initial_update.clone();
            let new_time = price_info_one_hour_before.starts_at - Duration::try_hours(1).unwrap();
            price_info_one_hour_before.starts_at = new_time;
            let updated_price_info_after_one_hour =
                update_current_energy_price_info(&config, Some(price_info_one_hour_before)).await;
            assert!(updated_price_info_after_one_hour.is_ok());
            assert_ne!(
                updated_price_info_after_one_hour.unwrap().starts_at,
                new_time
            );
        }

        #[tokio::test]
        #[serial]
        async fn test_loop_for_data() {
            let config = Config {
                access: AccessConfig::default(),
                output: OutputConfig::new(OutputType::Silent)
                    .with_display_mode(output::DisplayMode::Prices)
                    .with_gui_mode(output::GuiMode::Simple),
                logging: LogConfig::default(),
            };
            let mut subscription = Box::new(connect_live_measurement(&config.access).await);
            let app_state = create_app_state();

            let result = loop_for_data(&config, subscription.as_mut(), app_state.clone());
            tokio::time::sleep(TokioDuration::from_secs(10)).await;
            app_state.lock().unwrap().should_quit = true;
            let result = timeout(std::time::Duration::from_secs(30), result).await;
            assert!(result.is_ok());
            assert!(result.unwrap().is_ok());
            subscription.stop().await.unwrap();
        }

        #[tokio::test]
        #[serial]
        async fn test_loop_for_data_invalid_home_id() {
            let mut config = Config {
                access: AccessConfig::default(),
                output: OutputConfig::new(OutputType::Silent)
                    .with_display_mode(output::DisplayMode::Prices)
                    .with_gui_mode(output::GuiMode::Simple),
                logging: LogConfig::default(),
            };
            config.access.home_id.pop();
            let mut subscription = Box::new(connect_live_measurement(&config.access).await);
            let app_state = create_app_state();

            let result = timeout(
                std::time::Duration::from_secs(10),
                loop_for_data(&config, subscription.as_mut(), app_state),
            )
            .await;
            assert!(result.is_ok());
            let result = result.unwrap();
            assert!(result.as_ref().is_err());
            let error = result.err().unwrap();
            let error_type = error.downcast::<LoopEndingError>();
            assert!(error_type.is_ok());
            assert_eq!(*error_type.unwrap(), LoopEndingError::InvalidData);
            subscription.stop().await.unwrap();
        }

        #[tokio::test]
        #[serial]
        async fn test_loop_for_data_connection_timeout() {
            let mut config = Config {
                access: AccessConfig::default(),
                output: OutputConfig::new(OutputType::Silent)
                    .with_display_mode(output::DisplayMode::Prices),
                logging: LogConfig::default(),
            };
            config.access.reconnect_timeout = 0;
            let mut subscription = Box::new(connect_live_measurement(&config.access).await);
            let app_state = create_app_state();

            let result = loop_for_data(&config, subscription.as_mut(), app_state).await;
            assert!(result.as_ref().is_err());
            let error = result.err().unwrap();
            let error_type = error.downcast::<LoopEndingError>();
            assert!(error_type.is_ok());
            assert_eq!(*error_type.unwrap(), LoopEndingError::Reconnect);
            subscription.stop().await.unwrap();
        }

        #[tokio::test]
        #[serial]
        async fn test_fetch_display_data() {
            // Test Prices mode
            let config = Config {
                access: AccessConfig::default(),
                output: OutputConfig::new(OutputType::Silent)
                    .with_display_mode(output::DisplayMode::Prices),
                logging: LogConfig::default(),
            };

            let result =
                fetch_display_data(&config.access, &config.output.display_mode, &None).await;
            assert!(result.is_ok());
            let display_data = result.unwrap();
            assert!(display_data.is_some());

            if let Some((prices, description, _)) = display_data {
                assert_eq!(prices.len(), 24);
                assert_eq!(description, "Energy Prices [EUR/kWh]");
            }

            // Test Consumption mode
            let config = Config {
                access: AccessConfig::default(),
                output: OutputConfig::new(OutputType::Silent)
                    .with_display_mode(output::DisplayMode::Consumption),
                logging: LogConfig::default(),
            };

            let result =
                fetch_display_data(&config.access, &config.output.display_mode, &None).await;
            assert!(result.is_ok());
            let display_data = result.unwrap();
            assert!(display_data.is_some());

            if let Some((consumption, description, _)) = display_data {
                assert_eq!(consumption.len(), 24);
                assert_eq!(description, "Energy Consumption [kWh]");
            }
        }

        #[tokio::test]
        #[serial]
        async fn test_fetch_display_data_cost_mode() {
            // Test Cost mode
            let config = Config {
                access: AccessConfig::default(),
                output: OutputConfig::new(OutputType::Silent)
                    .with_display_mode(output::DisplayMode::Cost),
                logging: LogConfig::default(),
            };

            let estimated_daily_fee = Some(24.0); // Example daily fee

            let result = fetch_display_data(
                &config.access,
                &config.output.display_mode,
                &estimated_daily_fee,
            )
            .await;
            assert!(result.is_ok());
            let display_data = result.unwrap();
            assert!(display_data.is_some());

            if let Some((costs, description, expiry_date)) = display_data {
                assert_eq!(costs.len(), 24);
                assert_eq!(description, "Cost [EUR]");
                // Check if the cost for the current hour is not zero (since we have a daily fee)
                let current_hour = Local::now().hour();
                let next_hour = Local::now()
                    .date_naive()
                    .and_hms_opt(current_hour + 1, 0, 0)
                    .unwrap()
                    .and_local_timezone(Local)
                    .unwrap()
                    .fixed_offset();

                assert!(next_hour == expiry_date);

                for i in 0..24 {
                    if i < current_hour as usize {
                        assert!(costs[i] > 0.0);
                    } else {
                        assert_eq!(costs[i], 0.0);
                    }
                }
            }
        }

        #[tokio::test]
        #[serial]
        async fn test_cache_expired_no_cache() {
            let app_state = create_app_state();
            assert!(cache_expired(&app_state.lock().unwrap()));
        }

        #[tokio::test]
        #[serial]
        async fn test_cache_expired_cache_not_expired() {
            let app_state = create_app_state();
            let mut state = app_state.lock().unwrap();
            let timestamp = Local::now().fixed_offset() + chrono::Duration::minutes(1);
            state
                .cached_bar_graph
                .insert(DisplayMode::Prices, (vec![], String::from(""), timestamp));
            assert!(!cache_expired(&state));
        }

        #[tokio::test]
        #[serial]
        async fn test_cache_expired_cache_expired() {
            let app_state = create_app_state();
            let mut state = app_state.lock().unwrap();
            let timestamp = Local::now().fixed_offset() - chrono::Duration::hours(1);
            state
                .cached_bar_graph
                .insert(DisplayMode::Prices, (vec![], String::from(""), timestamp));
            assert!(cache_expired(&state));
        }

        #[tokio::test]
        #[serial]
        async fn test_cache_expired_different_display_mode() {
            let app_state = create_app_state();
            let mut state = app_state.lock().unwrap();
            state.display_mode = DisplayMode::Consumption;
            let timestamp = Local::now().fixed_offset();
            state
                .cached_bar_graph
                .insert(DisplayMode::Prices, (vec![], String::from(""), timestamp));
            assert!(cache_expired(&state));
        }
    }
}

pub mod html_logger;
