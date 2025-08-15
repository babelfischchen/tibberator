#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use chrono::{DateTime, Duration, FixedOffset, Timelike, Local};
    use serial_test::serial;
    use tokio::time::{timeout, Duration as TokioDuration};

    use crate::tibber::{
        AccessConfig, Config, LoopEndingError, PriceInfo, RealTibberDataProvider,
        update_current_energy_price_info, loop_for_data_with_provider,
        fetch_display_data_with_provider, check_user_shutdown, cache_expired,
        connect_live_measurement
    };
    use crate::tibber::output::{DisplayMode, GuiMode, OutputConfig, OutputType};
    use crate::tibber::tui::AppState;
    use crate::html_logger::LogConfig;

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
                .with_display_mode(DisplayMode::Consumption)
                .with_gui_mode(GuiMode::Simple),
            logging: LogConfig::default(),
        };
        let subscription_result = connect_live_measurement(&config.access).await;
        assert!(subscription_result.is_ok());
        let mut subscription = Box::new(subscription_result.unwrap());
        let app_state = create_app_state();

        let provider = RealTibberDataProvider;
        let result = loop_for_data_with_provider(
            &config,
            subscription.as_mut(),
            app_state.clone(),
            &provider,
        );
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
                .with_display_mode(DisplayMode::Consumption)
                .with_gui_mode(GuiMode::Simple),
            logging: LogConfig::default(),
        };
        config.access.home_id.pop();
        let subscription_result = connect_live_measurement(&config.access).await;
        assert!(subscription_result.is_ok());
        let mut subscription = Box::new(subscription_result.unwrap());
        let app_state = create_app_state();

        let provider = RealTibberDataProvider;
        let result = timeout(
            std::time::Duration::from_secs(10),
            loop_for_data_with_provider(&config, subscription.as_mut(), app_state, &provider),
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
                .with_display_mode(DisplayMode::Consumption),
            logging: LogConfig::default(),
        };
        config.access.reconnect_timeout = 0;
        let subscription_result = connect_live_measurement(&config.access).await;
        assert!(subscription_result.is_ok());
        let mut subscription = Box::new(subscription_result.unwrap());
        let app_state = create_app_state();

        let provider = RealTibberDataProvider;
        let result =
            loop_for_data_with_provider(&config, subscription.as_mut(), app_state, &provider)
                .await;
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
        // Test Consumption mode
        let config = Config {
            access: AccessConfig::default(),
            output: OutputConfig::new(OutputType::Silent)
                .with_display_mode(DisplayMode::Consumption),
            logging: LogConfig::default(),
        };

        let result = fetch_display_data_with_provider(
            &RealTibberDataProvider,
            &config.access,
            &config.output.display_mode,
            &None,
        )
        .await;
        assert!(result.is_ok());
        let display_data = result.unwrap();
        assert!(display_data.is_some());

        if let Some((consumption, description, _)) = display_data {
            assert_eq!(consumption.len(), 24);
            assert_eq!(description, "Energy Consumption [kWh]");
        }

        // Test Cost mode
        let config = Config {
            access: AccessConfig::default(),
            output: OutputConfig::new(OutputType::Silent)
                .with_display_mode(DisplayMode::Cost),
            logging: LogConfig::default(),
        };

        let result = fetch_display_data_with_provider(
            &RealTibberDataProvider,
            &config.access,
            &config.output.display_mode,
            &None,
        )
        .await;
        assert!(result.is_ok());
        let display_data = result.unwrap();
        assert!(display_data.is_some());

        if let Some((cost, description, _)) = display_data {
            assert_eq!(cost.len(), 24);
            assert_eq!(description, "Cost Today [EUR]");
        }
    }

    #[tokio::test]
    #[serial]
    async fn test_check_user_shutdown_timeout() {
        use std::sync::mpsc::channel;

        let (_sender, receiver) = channel();

        // Test timeout scenario - no value sent
        let result = check_user_shutdown(&receiver);
        assert_eq!(result, false);
    }

    #[tokio::test]
    #[serial]
    async fn test_check_user_shutdown_disconnect() {
        use std::sync::mpsc::channel;

        let (sender, receiver) = channel::<bool>();
        drop(sender); // Drop the sender to simulate disconnection

        // Test disconnection scenario
        let result = check_user_shutdown(&receiver);
        assert_eq!(result, true);
    }

    #[tokio::test]
    #[serial]
    async fn test_check_user_shutdown_normal() {
        use std::sync::mpsc::channel;

        let (sender, receiver) = channel();

        // Test normal scenario - value sent
        sender.send(true).unwrap();
        let result = check_user_shutdown(&receiver);
        assert_eq!(result, true);
    }

    #[tokio::test]
    #[serial]
    async fn test_fetch_display_data_all_modes() {
        let config = Config {
            access: AccessConfig::default(),
            output: OutputConfig::new(OutputType::Silent)
                .with_display_mode(DisplayMode::Consumption),
            logging: LogConfig::default(),
        };

        let provider = RealTibberDataProvider;

        // Test all display modes
        let display_modes = vec![
            DisplayMode::Consumption,
            DisplayMode::Cost,
            DisplayMode::CostLast30Days,
            DisplayMode::CostLast12Months,
            DisplayMode::AllYears,
        ];

        for mode in display_modes {
            let result =
                fetch_display_data_with_provider(&provider, &config.access, &mode, &None).await;

            // All should return Ok
            assert!(result.is_ok());

            // Uncomment the following lines if you want to check that data is returned
            // let display_data = result.unwrap();
            // assert!(display_data.is_some());
        }
    }

    #[tokio::test]
    #[serial]
    async fn test_fetch_display_data_cost_mode() {
        // Test Cost mode
        let config = Config {
            access: AccessConfig::default(),
            output: OutputConfig::new(OutputType::Silent)
                .with_display_mode(DisplayMode::Cost),
            logging: LogConfig::default(),
        };

        let estimated_daily_fee = Some(24.0); // Example daily fee

        let result = fetch_display_data_with_provider(
            &RealTibberDataProvider,
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
            assert_eq!(description, "Cost Today [EUR]");
            // Check if the cost for the current hour is not zero (since we have a daily fee)

            let offset = if Local::now().minute() < 15 { 0 } else { 1 };
            let current_hour = Local::now().hour();
            let next_hour = Local::now()
                .date_naive()
                .and_hms_opt(current_hour, 15, 0)
                .unwrap()
                .and_local_timezone(Local)
                .unwrap()
                .fixed_offset()
                + chrono::Duration::hours(offset);

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

    #[tokio::test]
    #[serial]
    async fn test_cache_expired_empty_cache() {
        let app_state = create_app_state();
        let mut state = app_state.lock().unwrap();
        state.display_mode = DisplayMode::Prices;
        // Test with empty cache
        assert!(cache_expired(&state));
    }

    #[tokio::test]
    #[serial]
    async fn test_cache_expired_not_expired() {
        let app_state = create_app_state();
        let mut state = app_state.lock().unwrap();
        state.display_mode = DisplayMode::Prices;
        // Test with cache not expired (future timestamp)
        let timestamp = Local::now().fixed_offset() + chrono::Duration::hours(1);
        state
            .cached_bar_graph
            .insert(DisplayMode::Prices, (vec![], String::from(""), timestamp));
        assert!(!cache_expired(&state));
    }
}