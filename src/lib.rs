//! Module for easy access to the Tibber API
//!
//! This module contains various helper methods to connect to the Tibber API (see https://developer.tibber.com).
//! You need an access token in order to use the API.

#[cfg(test)]
#[path = "lib_tests/mod.rs"]
mod lib_tests;

pub mod tibber {
    use async_trait::async_trait;
    use chrono::{DateTime, FixedOffset, Local};
    use futures::{future, stream::StreamExt, task::Poll};
    use log::{debug, error, info};
    use serde::{Deserialize, Serialize};
    use std::{
        cell::Cell,
        rc::Rc,
        sync::mpsc::{Receiver, RecvTimeoutError},
        sync::{Arc, Mutex},
        time::Instant,
    };

    // Import specific items from data_handling and data_types that are needed
    use crate::html_logger::LogConfig;
    pub use crate::tibber::data_handling::{
        check_real_time_subscription, connect_live_measurement, estimate_daily_fees,
        get_consumption_data_today, get_cost_all_years, get_cost_data_today,
        get_cost_last_12_months, get_cost_last_30_days, get_home_ids, get_prices_today_tomorrow,
        live_measurement, update_current_energy_price_info, LiveMeasurementOperation,
        LiveMeasurementSubscription,
    };
    pub use crate::tibber::data_types::{AccessConfig, LoopEndingError, PriceInfo};
    pub use crate::tibber::output::OutputConfig as OutputConfigType;
    pub use crate::tibber::tui::AppState;
    use output::{print_screen, DisplayMode};

    // Add trait definition for TibberDataProvider
    #[async_trait::async_trait]
    pub trait TibberDataProvider {
        async fn get_consumption_data_today(
            &self,
            config: &AccessConfig,
        ) -> Result<
            Option<(Vec<f64>, String, DateTime<FixedOffset>)>,
            Box<dyn std::error::Error + Send + Sync>,
        >;

        async fn get_cost_data_today(
            &self,
            config: &AccessConfig,
            estimated_daily_fee: &Option<f64>,
        ) -> Result<
            Option<(Vec<f64>, String, DateTime<FixedOffset>)>,
            Box<dyn std::error::Error + Send + Sync>,
        >;

        async fn get_cost_last_30_days(
            &self,
            config: &AccessConfig,
            estimated_daily_fee: &Option<f64>,
        ) -> Result<
            Option<(Vec<f64>, String, DateTime<FixedOffset>)>,
            Box<dyn std::error::Error + Send + Sync>,
        >;

        async fn get_cost_last_12_months(
            &self,
            config: &AccessConfig,
            estimated_daily_fee: &Option<f64>,
        ) -> Result<
            Option<(Vec<f64>, String, DateTime<FixedOffset>)>,
            Box<dyn std::error::Error + Send + Sync>,
        >;

        async fn get_cost_all_years(
            &self,
            config: &AccessConfig,
            estimated_daily_fee: &Option<f64>,
        ) -> Result<
            Option<(Vec<f64>, String, DateTime<FixedOffset>)>,
            Box<dyn std::error::Error + Send + Sync>,
        >;

        async fn get_prices_today_tomorrow(
            &self,
            config: &AccessConfig,
        ) -> Result<
            Option<((Vec<f64>, Vec<f64>), String, DateTime<FixedOffset>)>,
            Box<dyn std::error::Error + Send + Sync>,
        >;

        async fn estimate_daily_fees(
            &self,
            config: &AccessConfig,
        ) -> Result<Option<f64>, Box<dyn std::error::Error + Send + Sync>>;

        async fn connect_live_measurement(
            &self,
            config: &AccessConfig,
        ) -> Result<LiveMeasurementSubscription, Box<dyn std::error::Error + Send + Sync>>;
    }

    // Add real implementation
    pub struct RealTibberDataProvider;

    #[async_trait]
    impl TibberDataProvider for RealTibberDataProvider {
        async fn get_consumption_data_today(
            &self,
            config: &AccessConfig,
        ) -> Result<
            Option<(Vec<f64>, String, DateTime<FixedOffset>)>,
            Box<dyn std::error::Error + Send + Sync>,
        > {
            get_consumption_data_today(config).await
        }

        async fn get_cost_data_today(
            &self,
            config: &AccessConfig,
            estimated_daily_fee: &Option<f64>,
        ) -> Result<
            Option<(Vec<f64>, String, DateTime<FixedOffset>)>,
            Box<dyn std::error::Error + Send + Sync>,
        > {
            get_cost_data_today(config, estimated_daily_fee).await
        }

        async fn get_cost_last_30_days(
            &self,
            config: &AccessConfig,
            estimated_daily_fee: &Option<f64>,
        ) -> Result<
            Option<(Vec<f64>, String, DateTime<FixedOffset>)>,
            Box<dyn std::error::Error + Send + Sync>,
        > {
            get_cost_last_30_days(config, estimated_daily_fee).await
        }

        async fn get_cost_last_12_months(
            &self,
            config: &AccessConfig,
            estimated_daily_fee: &Option<f64>,
        ) -> Result<
            Option<(Vec<f64>, String, DateTime<FixedOffset>)>,
            Box<dyn std::error::Error + Send + Sync>,
        > {
            get_cost_last_12_months(config, estimated_daily_fee).await
        }

        async fn get_cost_all_years(
            &self,
            config: &AccessConfig,
            estimated_daily_fee: &Option<f64>,
        ) -> Result<
            Option<(Vec<f64>, String, DateTime<FixedOffset>)>,
            Box<dyn std::error::Error + Send + Sync>,
        > {
            get_cost_all_years(config, estimated_daily_fee).await
        }

        async fn get_prices_today_tomorrow(
            &self,
            config: &AccessConfig,
        ) -> Result<
            Option<((Vec<f64>, Vec<f64>), String, DateTime<FixedOffset>)>,
            Box<dyn std::error::Error + Send + Sync>,
        > {
            get_prices_today_tomorrow(config).await
        }

        async fn estimate_daily_fees(
            &self,
            config: &AccessConfig,
        ) -> Result<Option<f64>, Box<dyn std::error::Error + Send + Sync>> {
            estimate_daily_fees(config).await
        }

        async fn connect_live_measurement(
            &self,
            config: &AccessConfig,
        ) -> Result<LiveMeasurementSubscription, Box<dyn std::error::Error + Send + Sync>> {
            connect_live_measurement(config).await
        }
    }

    mod data_handling;
    mod data_types;
    pub mod output;
    pub mod tui;

    #[derive(Debug, Serialize, Deserialize)]
    pub struct Config {
        pub access: AccessConfig,
        pub output: OutputConfigType,
        pub logging: LogConfig,
    }

    impl Default for Config {
        fn default() -> Self {
            Config {
                access: AccessConfig::default(),
                output: OutputConfigType::default(),
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
    /// ```no_run
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

    /// Asynchronously processes streaming data from a subscription, monitoring for user shutdown requests
    /// and reconnect conditions. The function takes care of handling timeouts, disconnections, and invalid data.
    ///
    /// ## Parameters
    /// - `config`: A reference to the configuration settings.
    /// - `subscription`: A mutable reference to the data subscription.
    /// - `app_state`: A shared reference to the application state for updating the UI.
    /// - `provider`: A reference to a TibberDataProvider trait object for fetching additional data.
    ///
    /// ## Return Value
    /// - `Ok(())`: Indicates successful completion (shutdown requested).
    /// - `Err(LoopEndingError::Reconnect)`: Indicates the need to reconnect due to elapsed time.
    /// - `Err(LoopEndingError::InvalidData)`: Indicates no valid data received during the loop.
    ///
    /// ## Example
    /// ```no_run
    ///   use tibberator::tibber::{
    ///                            tui::AppState,
    ///                            Config, loop_for_data_with_provider,
    ///                            AccessConfig, connect_live_measurement, RealTibberDataProvider,
    ///                           };
    ///   use std::sync::{Arc, Mutex};
    ///   use tokio::time;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    ///   let config = Config::default();
    ///   let subscription_result = connect_live_measurement(&config.access).await;
    ///   // Handle the Result properly
    ///   if let Ok(mut subscription) = subscription_result {
    ///     let app_state = Arc::new(Mutex::new(AppState::default()));
    ///     let provider = RealTibberDataProvider;
    ///
    ///     let state = app_state.clone();
    ///
    ///     tokio::spawn(async move {
    ///       tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
    ///       app_state.lock().unwrap().should_quit = true;
    ///     });
    ///     let result = loop_for_data_with_provider(&config, &mut subscription, state, &provider).await;
    ///     assert!(result.is_ok());
    ///   }
    /// # }
    /// ```
    pub async fn loop_for_data_with_provider(
        config: &Config,
        subscription: &mut LiveMeasurementSubscription,
        app_state: Arc<Mutex<AppState>>,
        provider: &dyn TibberDataProvider,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
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
        let display_data = match &config.output.display_mode {
            output::DisplayMode::Prices | output::DisplayMode::PricesTomorrow => {
                // Handle Prices display modes directly
                match fetch_prices_display_data(&config.access).await? {
                    Some(((today_prices, tomorrow_prices), description, timestamp)) => {
                        // Select the appropriate prices based on the display mode
                        let selected_prices = match &config.output.display_mode {
                            output::DisplayMode::Prices => today_prices,
                            output::DisplayMode::PricesTomorrow => tomorrow_prices,
                            // These cases are already handled by the outer match, but included for completeness
                            _ => today_prices,
                        };
                        Some((selected_prices, description, timestamp))
                    }
                    None => None,
                }
            }
            // For all other display modes, use the existing fetch function
            _ => {
                fetch_display_data_with_provider(
                    provider,
                    &config.access,
                    &config.output.display_mode,
                    &None,
                )
                .await?
            }
        };

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
    /// Fetches display data based on the specified display mode.
    ///
    /// # Arguments
    /// * `access_config` - A reference to the access configuration for fetching data.
    /// * `display_mode` - A reference to the display mode indicating what type of data to fetch (prices, consumption, or cost).
    /// * `estimated_daily_fee` - An optional reference to the estimated daily fee used in cost calculations.
    ///
    /// # Returns
    /// * `Result<Option<(Vec<f64>, String, DateTime<FixedOffset>)>, Box<dyn std::error::Error>>`
    ///   - `Ok(Some((prices, consumption_data, timestamp)))` if data is successfully fetched and contains prices, consumption data, and a timestamp.
    ///   - `Ok(None)` if no data is available.
    ///   - `Err(e)` if an error occurs during the fetch operation.
    ///
    /// # Examples
    /// ```no_run
    /// use tibberator::html_logger::LogConfig;
    /// use tibberator::tibber::{output::{self, GuiMode, OutputConfig, OutputType, DisplayMode}, AccessConfig, Config, fetch_display_data_with_provider, RealTibberDataProvider};
    /// use chrono::FixedOffset;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// #    let mut config = Config::default();
    /// #    config.output.display_mode = DisplayMode::Consumption;  // Use a supported display mode
    /// #    let estimated_daily_fee = Some(10.5);
    /// #    let provider = RealTibberDataProvider;
    /// #
    /// #    match fetch_display_data_with_provider(&provider, &config.access, &config.output.display_mode, &estimated_daily_fee).await {
    /// #        Ok(Some((prices, consumption_data, timestamp))) => {
    /// #            println!("Prices: {:?}", prices);
    /// #            println!("Consumption Data: {:?}", consumption_data);
    /// #            println!("Timestamp: {:?}", timestamp);
    /// #        }
    /// #        Ok(None) => println!("No data available."),
    /// #        Err(e) => eprintln!("Error fetching display data: {}", e),
    /// #    }
    /// # }
    /// ```
    pub async fn fetch_display_data_with_provider(
        provider: &dyn TibberDataProvider,
        access_config: &AccessConfig,
        display_mode: &DisplayMode,
        estimated_daily_fee: &Option<f64>,
    ) -> Result<
        Option<(Vec<f64>, String, DateTime<FixedOffset>)>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        match display_mode {
            output::DisplayMode::Consumption => {
                provider.get_consumption_data_today(access_config).await
            }
            output::DisplayMode::Cost => {
                provider
                    .get_cost_data_today(access_config, estimated_daily_fee)
                    .await
            }
            output::DisplayMode::CostLast30Days => {
                provider
                    .get_cost_last_30_days(access_config, estimated_daily_fee)
                    .await
            }
            output::DisplayMode::CostLast12Months => {
                provider
                    .get_cost_last_12_months(access_config, estimated_daily_fee)
                    .await
            }
            output::DisplayMode::AllYears => {
                provider
                    .get_cost_all_years(access_config, estimated_daily_fee)
                    .await
            }
            _ => {
                panic!("Not implemented fetching for {:?}", display_mode)
            }
        }
    }

    /// Fetches today's and tomorrow's energy prices for display purposes.
    ///
    /// This function retrieves hourly energy prices for both the current day and the next day
    /// from the Tibber API. The data is formatted for display in the application's UI.
    ///
    /// # Arguments
    ///
    /// * `access_config` - A reference to the access configuration containing the necessary credentials.
    ///
    /// # Returns
    ///
    /// * `Result<Option<((Vec<f64>, Vec<f64>), String, DateTime<FixedOffset>)>, Box<dyn std::error::Error>>`
    ///   - `Ok(Some(((today_prices, tomorrow_prices), description, expiration_time)))` containing
    ///     energy prices in EUR/kWh for today and tomorrow, a description string, and the expiration time.
    ///   - `Ok(None)` if no data could be fetched or processed.
    ///   - `Err(error)` if there was a problem fetching or processing the energy prices.
    ///
    pub async fn fetch_prices_display_data(
        access_config: &AccessConfig,
    ) -> Result<
        Option<((Vec<f64>, Vec<f64>), String, DateTime<FixedOffset>)>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        get_prices_today_tomorrow(access_config).await
    }

    /// Estimates the daily fees using a TibberDataProvider.
    ///
    /// This function provides a wrapper around the TibberDataProvider's estimate_daily_fees method,
    /// allowing for consistent fee estimation across different implementations of the trait.
    ///
    /// # Arguments
    ///
    /// * `provider` - A reference to a TibberDataProvider trait object that will be used to estimate fees.
    /// * `config` - A reference to the access configuration containing necessary information.
    ///
    /// # Returns
    ///
    /// * `Result<Option<f64>, Box<dyn std::error::Error>>`
    ///   - `Ok(Some(fee))` if the daily fee was successfully estimated.
    ///   - `Ok(None)` if no data is available to calculate fees.
    ///   - `Err(error)` if an error occurred during the estimation process.
    ///
    pub async fn estimate_daily_fees_with_provider(
        provider: &dyn TibberDataProvider,
        config: &AccessConfig,
    ) -> Result<Option<f64>, Box<dyn std::error::Error + Send + Sync>> {
        provider.estimate_daily_fees(config).await
    }
}

pub mod html_logger;
