//! Tests for the data handling module

#[cfg(test)]
mod tests {
    use super::super::*;
    use serial_test::serial;
    use tokio::time::{timeout, Duration as TokioDuration};

    #[tokio::test]
    async fn test_fetch_data() {
        let config = AccessConfig::default();

        let result = fetch_home_data(&config).await;

        assert!(result.is_ok());
        let response = result.unwrap();
        assert!(response.errors.is_none());

        let response_data = response.data;
        assert!(response_data.is_some());
        let response_data = response_data.unwrap();

        let owner = response_data.viewer.home.owner;
        let features = response_data.viewer.home.features;
        assert!(owner.is_some());
        assert_eq!(owner.unwrap().name, "Arya Stark");
        assert!(features.is_some());
        assert!(features.unwrap().real_time_consumption_enabled.unwrap());
    }

    #[tokio::test]
    async fn test_fetch_subscription_url() {
        let config = AccessConfig::default();

        let result = fetch_subscription_url(&config).await;
        assert!(result.is_ok());
        let url_data = result.unwrap();
        assert_eq!(
            url_data,
            "wss://websocket-api.tibber.com/v1-beta/gql/subscriptions"
        );
    }

    #[tokio::test]
    async fn test_get_home_ids() {
        let config = AccessConfig::default();
        let result = get_home_ids(&config).await;
        assert!(result.is_ok());
        let home_ids = result.unwrap();
        assert_eq!(home_ids.len(), 1);
        let home_id = home_ids.last();
        assert!(home_id.is_some());
        assert_eq!(home_id.unwrap(), "96a14971-525a-4420-aae9-e5aedaa129ff");
    }

    #[tokio::test]
    async fn test_get_price_current() {
        let config = AccessConfig::default();
        let id = config.home_id.to_owned();
        let variables = price_current::Variables { id };

        let result = fetch_data::<PriceCurrent>(&config, variables).await;
        assert!(result.is_ok());

        let result2 = get_current_energy_price(&config).await;
        assert!(result2.is_ok());

        let result3 = update_current_energy_price_info(&config, None).await;
        assert!(result3.is_ok());
    }

    #[tokio::test]
    async fn test_create_websocket() {
        let config = AccessConfig::default();

        let result = create_websocket(&config).await;
        assert!(result.is_ok());
        let (mut test_instance, _) = result.unwrap();
        assert!(test_instance.close(None).await.is_ok());
    }

    #[tokio::test]
    #[serial]
    async fn test_get_live_measurement() {
        use futures::stream::StreamExt;
        let config = AccessConfig::default();

        let mut subscription = connect_live_measurement(&config).await.unwrap();

        for _ in 1..=5 {
            let result = timeout(std::time::Duration::from_secs(90), subscription.next()).await;
            assert!(result.is_ok());
            let item = result.unwrap();
            if item.is_none() {
                break;
            }

            let item = item
                .unwrap()
                .unwrap()
                .data
                .unwrap()
                .live_measurement
                .unwrap();
            println!("{:?} => {:?}", item.timestamp, item.power);
            assert!(item.power >= 0.);
            assert!(item.accumulated_consumption > 0.);
        }

        let stop_result = subscription.stop().await;
        assert!(stop_result.is_ok());
    }

    #[tokio::test]
    async fn test_get_todays_energy_consumption() {
        let config = AccessConfig::default();

        tokio::time::sleep(TokioDuration::from_secs(10)).await;
        let result = get_todays_energy_consumption(&config).await;
        assert!(result.is_ok());

        let consumption_nodes = result.unwrap();
        assert_eq!(consumption_nodes.len(), 24, "Should have 24 hourly entries");

        let current_time = Utc::now();
        for node in consumption_nodes.into_iter() {
            let converted_node_time = node.from.with_timezone(&Utc);

            if converted_node_time >= current_time {
                assert_eq!(
                    node.consumption, 0.0,
                    "Consumption should be 0 for future hours"
                );
                assert_eq!(node.cost, 0.0, "Cost should be 0 for future hours");
            } else {
                assert!(
                    node.consumption >= 0.0,
                    "Consumption should be non-negative"
                );
                assert!(node.cost >= 0.0, "Cost should be non-negative");
            }
        }
    }

    #[tokio::test]
    async fn test_get_consumption_page() {
        let config = AccessConfig::default();

        tokio::time::sleep(TokioDuration::from_secs(15)).await;
        let result = get_consumption_page(&config, &String::from("")).await;
        assert!(result.is_ok());

        let (consumption_page, _) = result.unwrap();
        assert_eq!(consumption_page.count, 1, "Should have 1 page entries");
        assert_ne!(consumption_page.currency, "");
        assert!(consumption_page.has_previous_page);
        assert_ne!(consumption_page.start_cursor, "");
        assert!(consumption_page.total_consumption >= 0.0);
        assert!(consumption_page.total_cost >= 0.0);
    }

    #[tokio::test]
    async fn test_get_last_10_hours_consumption() {
        use std::collections::HashSet;

        let config = AccessConfig::default();
        tokio::time::sleep(TokioDuration::from_secs(20)).await;
        let mut result = get_consumption_page(&config, &String::from("")).await;
        assert!(result.is_ok());

        let mut pages = Vec::new();

        for _i in 0..10 {
            let (consumption, _) = result.unwrap();
            assert_eq!(consumption.count, 1, "Should have 1 page entries");
            assert!(consumption.has_previous_page);
            assert_ne!(consumption.start_cursor, "", "Cursor must be valid");
            result = get_consumption_page(&config, &consumption.start_cursor).await;

            pages.push(consumption);
        }

        assert_eq!(pages.len(), 10, "There should be 10 pages");

        // check that all 10 pages are unique
        let mut seen_consumptions = HashSet::new();
        for consumption in &pages {
            let cursor = &consumption.start_cursor;
            if !seen_consumptions.insert(cursor) {
                panic!("Duplicate consumption found: {:?}", consumption);
            }
        }
        assert!(
            seen_consumptions.len() == pages.len(),
            "All pages should be unique"
        );
    }

    #[tokio::test]
    async fn test_estimate_daily_fee() {
        let config = AccessConfig::default();
        let estimated_fee = estimate_daily_fees(&config).await;
        assert!(estimated_fee.is_ok());

        let estimated_fee = estimated_fee.unwrap();
        assert!(estimated_fee.is_some());

        assert!(estimated_fee.unwrap() >= 0.0);
    }

    #[tokio::test]
    async fn test_get_cost_all_years() {
        let config = AccessConfig::default();
        let estimated_daily_fee = None; // You can set this to an actual value if needed

        tokio::time::sleep(TokioDuration::from_secs(25)).await;
        let result = get_cost_all_years(&config, &estimated_daily_fee).await;
        assert!(result.is_ok());

        let (yearly_nodes, description, time) = result.unwrap().unwrap();
        assert_eq!(description, "Yearly Cost [EUR]");

        let reference_time = Local::now()
            .with_day(1)
            .and_then(|t| t.with_month(1))
            .unwrap()
            .fixed_offset();
        assert!(time > reference_time, "Time should not be zero");
        assert!(!yearly_nodes.is_empty());
    }

    #[tokio::test]
    async fn test_price_info_invalid_data() {
        // Test that PriceInfo parsing returns None when data is missing or malformed
        let invalid_data = price_current::PriceCurrentViewerHomeCurrentSubscriptionPriceInfoCurrent {
            total: None,
            energy: None,
            tax: None,
            starts_at: None,
            currency: String::from("EUR"),
            level: Some(price_current::PriceLevel::NORMAL),
        };

        let result = PriceInfo::new_current(invalid_data);
        assert!(result.is_none(), "Parsing should return None for invalid data");
    }
}