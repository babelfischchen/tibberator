mod mock_subscription;

#[cfg(test)]
mod lib_tests {
    use std::{env, future::IntoFuture, time::Duration};

    use async_tungstenite::{async_std::connect_async, tungstenite::client::IntoClientRequest};
    use futures::StreamExt;

    use crate::mock_subscription::{self, SubscriptionServer};
    use graphql_ws_client::Client;
    use http::HeaderValue;
    use std::sync::{Arc, Mutex};
    use tibberator::tibber::{
        live_measurement, loop_for_data, output::DisplayMode, tui::AppState, Config,
        LiveMeasurementOperation, LiveMeasurementSubscription,
    };
    use tokio::time::sleep;

    fn build_query(power: f64) -> mock_subscription::LiveMeasurement {
        mock_subscription::LiveMeasurement {
            power,
            ..Default::default()
        }
    }

    fn build_streaming_operation() -> LiveMeasurementOperation {
        let variables = live_measurement::Variables {
            id: "123".to_string(),
        };
        LiveMeasurementOperation::new(variables)
    }

    fn get_test_config() -> Config {
        let current_dir = env::current_dir().expect("Failed to get current directory.");
        let filename = "tests/test-config.toml";
        let path = current_dir.join(filename);
        confy::load_path(path).expect("Config file not found.")
    }

    async fn start_subscribtion(
        server: &SubscriptionServer,
    ) -> Result<LiveMeasurementSubscription, graphql_ws_client::Error> {
        let mut request = server.websocket_url().into_client_request().unwrap();
        request.headers_mut().insert(
            "Sec-WebSocket-Protocol",
            HeaderValue::from_str("graphql-transport-ws").unwrap(),
        );

        let (connection, _) = connect_async(request).await.unwrap();
        println!("Connected!");

        let (client, actor) = Client::build(connection).await.unwrap();
        tokio::spawn(actor.into_future());
        client.subscribe(build_streaming_operation()).await
    }

    #[tokio::test]
    async fn mock_server_test() {
        // ### assemble
        let server = SubscriptionServer::start().await;
        sleep(Duration::from_millis(50)).await;

        let stream = start_subscribtion(&server).await.unwrap();
        sleep(Duration::from_millis(100)).await;

        let updates = [build_query(20.0), build_query(40.0)];

        futures::join!(
            // ### act
            async {
                for update in &updates {
                    server.send(update.to_owned()).unwrap();
                }
            },
            // ### assert
            async {
                let received_updates = stream
                    .take(updates.len())
                    .collect::<Vec<
                        Result<
                            graphql_client::Response<live_measurement::ResponseData>,
                            graphql_ws_client::Error,
                        >,
                    >>()
                    .await;
                for (expected, received) in updates.iter().zip(received_updates) {
                    let received = received.unwrap();
                    assert!(received.errors.is_none());
                    let data = received.data.unwrap();
                    assert_eq!(data.live_measurement.unwrap().power, expected.power);
                }
            }
        );
    }

    #[tokio::test]
    async fn test_loop_for_data() {
        let server = SubscriptionServer::start().await;
        sleep(Duration::from_millis(50)).await;

        let mut stream = start_subscribtion(&server).await.unwrap();
        sleep(Duration::from_millis(100)).await;

        let app_state = Arc::new(Mutex::new(AppState {
            should_quit: false,
            measurement: None,
            price_info: None,
            estimated_daily_fees: None,
            bar_graph_data: None,
            display_mode: DisplayMode::Prices,
            status: String::from("Waiting for data..."),
            data_needs_refresh: false,
        }));

        let config = get_test_config();
        assert!(config.output.is_silent());

        let updates = [build_query(20.0), build_query(40.0), build_query(60.)];

        futures::join!(
            async {
                for update in &updates {
                    server.send(update.to_owned()).unwrap();
                    sleep(Duration::from_secs(3)).await;
                }
                sleep(Duration::from_secs(1)).await;
                app_state.lock().unwrap().should_quit = true;
            },
            async {
                let result = loop_for_data(&config, &mut stream, app_state.clone()).await;
                assert!(result.is_ok());
            }
        );
    }
}
