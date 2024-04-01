//! Module for easy access to the Tibber API
//!
//! This module contains various helper methods to connect to the Tibber API (see https://developer.tibber.com).
//! You need an access token in order to use the API.
pub mod tibber {
    use futures::{future, stream::StreamExt, task::Poll};
    use graphql_ws_client::{graphql::StreamingOperation, Subscription};
    use serde::{Deserialize, Serialize};
    use std::{
        cell::Cell,
        rc::Rc,
        sync::mpsc::{Receiver, RecvTimeoutError},
        time::Instant,
    };

    use data_handling::*;
    use output::*;

    pub mod data_handling {
        use async_tungstenite::{
            async_std::{connect_async, ConnectStream},
            tungstenite::handshake::client::{generate_key, Response},
            WebSocketStream,
        };
        use chrono::{DateTime, Duration, FixedOffset, Utc};
        use crossterm::style;
        use graphql_client::{reqwest::post_graphql, GraphQLQuery};
        use graphql_ws_client::{graphql::StreamingOperation, Client as WSClient, Subscription};
        use http::{request::Builder, Request, Uri};
        use reqwest::{
            header::{HeaderMap, AUTHORIZATION},
            Client,
        };
        use serde::{Deserialize, Serialize};
        use std::str::FromStr;
        use thiserror::Error;

        #[derive(GraphQLQuery)]
        #[graphql(
            schema_path = "tibber/schema.json",
            query_path = "tibber/home.graphql",
            respone_derives = "Debug"
        )]
        pub struct Home;

        #[derive(GraphQLQuery)]
        #[graphql(
            schema_path = "tibber/schema.json",
            query_path = "tibber/view.graphql",
            respone_derives = "Debug"
        )]
        pub struct Viewer;

        #[derive(GraphQLQuery)]
        #[graphql(
            schema_path = "tibber/schema.json",
            query_path = "tibber/livemeasurement.graphql",
            respone_derives = "Debug"
        )]
        pub struct LiveMeasurement;

        #[derive(GraphQLQuery)]
        #[graphql(
            schema_path = "tibber/schema.json",
            query_path = "tibber/price_current.graphql",
            respone_derives = "Debug"
        )]
        struct PriceCurrent;

        #[derive(Debug, Error, PartialEq)]
        pub enum LoopEndingError {
            #[error("Shutdown requested")]
            Shutdown,
            #[error("Connection timed out. Reconnection necessary.")]
            Reconnect,
            #[error("Invalid or no data received.")]
            InvalidData,
        }

        #[derive(Debug, Serialize, Deserialize)]
        pub struct AccessConfig {
            pub token: String,
            url: String,
            pub home_id: String,
            pub reconnect_timeout: u64,
        }

        impl Default for AccessConfig {
            fn default() -> Self {
                AccessConfig {
                    token: "5K4MVS-OjfWhK_4yrjOlFe1F6kJXPVf7eQYggo8ebAE".to_string(),
                    url: "https://api.tibber.com/v1-beta/gql".to_string(),
                    home_id: "96a14971-525a-4420-aae9-e5aedaa129ff".to_string(),
                    reconnect_timeout: 120,
                }
            }
        }

        #[derive(Debug, Clone, PartialEq)]
        pub enum PriceLevel {
            VeryCheap,
            Cheap,
            Normal,
            Expensive,
            VeryExpensive,
            Other(String),
            None,
        }

        impl Default for PriceLevel {
            fn default() -> Self {
                PriceLevel::None
            }
        }

        #[derive(Debug, Clone, Default, PartialEq)]
        pub struct PriceInfo {
            pub total: f64,
            pub energy: f64,
            pub tax: f64,
            pub starts_at: DateTime<FixedOffset>,
            pub currency: String,
            pub level: PriceLevel,
        }

        impl PriceInfo {
            fn new_current(
                price_info: price_current::PriceCurrentViewerHomeCurrentSubscriptionPriceInfoCurrent,
            ) -> Option<Self> {
                let total = price_info.total?;
                let energy = price_info.energy?;
                let tax = price_info.tax?;
                let starts_at = chrono::DateTime::parse_from_rfc3339(
                    price_info.starts_at.ok_or("No timestamp").ok()?.as_str(),
                )
                .ok()?;

                let price_level = match price_info.level {
                    Some(price_current::PriceLevel::VERY_CHEAP) => PriceLevel::VeryCheap,
                    Some(price_current::PriceLevel::CHEAP) => PriceLevel::Cheap,
                    Some(price_current::PriceLevel::NORMAL) => PriceLevel::Normal,
                    Some(price_current::PriceLevel::EXPENSIVE) => PriceLevel::Expensive,
                    Some(price_current::PriceLevel::VERY_EXPENSIVE) => PriceLevel::VeryExpensive,
                    Some(price_current::PriceLevel::Other(string)) => PriceLevel::Other(string),
                    _ => PriceLevel::None,
                };

                Some(PriceInfo {
                    total,
                    energy,
                    tax,
                    starts_at,
                    currency: price_info.currency,
                    level: price_level,
                })
            }
        }

        impl PriceLevel {
            /// Converts a `PriceLevel` variant to a corresponding text color using crossterm.
            ///
            /// ## Arguments
            ///
            /// * `self`: The `PriceLevel` variant to convert.
            ///
            /// ## Returns
            ///
            /// * `Option<crossterm::style::Color>`: The text color associated with the given `PriceLevel`,
            ///   or `None` if no color is defined.
            ///
            pub fn to_color(&self) -> Option<crossterm::style::Color> {
                match self {
                    PriceLevel::VeryCheap => Some(style::Color::DarkGreen),
                    PriceLevel::Cheap => Some(style::Color::Green),
                    PriceLevel::Normal => Some(style::Color::Yellow),
                    PriceLevel::Expensive => Some(style::Color::Red),
                    PriceLevel::VeryExpensive => Some(style::Color::DarkRed),
                    _ => None,
                }
            }
        }

        /// # create_client
        ///
        /// The `create_client` function constructs an HTTP client for making requests with the specified access token.
        /// It sets up necessary headers, including the authorization header with the provided access token.
        ///
        /// ## Parameters
        /// - `access_token`: A string representing the access token used for authentication.
        ///
        /// ## Returns
        /// A `Result` containing either:
        /// - A configured `Client` object that can be used to make HTTP requests.
        /// - An error of type `reqwest::Error` if there are issues during client creation.
        ///
        fn create_client(access_token: &str) -> Result<Client, reqwest::Error> {
            let mut headers = HeaderMap::new();

            headers.insert(
                AUTHORIZATION,
                format!("Bearer {}", access_token)
                    .parse()
                    .expect("Access token formatting error."),
            );

            Client::builder()
                .user_agent("graphql-rust/0.10.0")
                .default_headers(headers)
                .build()
        }

        /// Fetches data from the Tibber API by sending a GraphQL-Query using
        /// the `config`-values and the GraphQL-Variables `variables` to send.
        async fn fetch_data<T>(
            config: &AccessConfig,
            variables: <T as GraphQLQuery>::Variables,
        ) -> Result<graphql_client::Response<<T as GraphQLQuery>::ResponseData>, reqwest::Error>
        where
            T: GraphQLQuery,
        {
            let client = create_client(&config.token)?;
            post_graphql::<T, _>(&client, &config.url, variables).await
        }

        /// Gets all `Home` data
        ///
        /// Retrieves all `Home` data from the Tibber schema.
        ///
        /// Requires an `access_token` configured in the provided `config` to fetch data.
        ///
        /// ## Parameters
        /// - `config`: An `AccessConfig` containing configuration details (e.g., access token).
        ///
        /// ## Returns
        /// A `Result` containing either:
        /// - A `graphql_client::Response` with the response data for the `Home` query.
        /// - An error of type `reqwest::Error` if there are issues during data retrieval.
        ///
        /// ## Example Usage
        ///
        /// ```
        ///   use tibberator::tibber::data_handling::{AccessConfig, fetch_home_data};
        ///
        /// # #[tokio::main]
        /// # async fn main() {
        ///   let config = AccessConfig::default();
        ///   let home_response = fetch_home_data(&config).await.is_ok();
        ///   assert!(home_response);
        /// # }
        /// ```
        pub async fn fetch_home_data(
            config: &AccessConfig,
        ) -> Result<graphql_client::Response<<Home as GraphQLQuery>::ResponseData>, reqwest::Error>
        {
            let id = config.home_id.to_owned();
            let variables = home::Variables { id };

            fetch_data::<Home>(config, variables).await
        }

        /// Creates and configures a http request in order to create a websocket connection.
        ///
        /// The `configure_request` function constructs an HTTP request builder with necessary headers.
        /// It sets up the authorization header with the provided access token and other required headers.
        ///
        /// ## Parameters
        /// - `access_token`: A string representing the access token used for authentication.
        /// - `uri`: A `Uri` object representing the target URL.
        ///
        /// ## Returns
        /// A `Builder` object that can be further customized before building the final request.
        ///
        /// # Panics
        ///
        /// This function will panic if the `access_token` is ill-formed and cannot be parsed into a string.
        ///
        fn configure_request(access_token: &str, uri: Uri) -> Builder {
            let mut request_builder = Request::builder()
                .uri(uri.to_string())
                .header("Host", uri.host().unwrap())
                .header("Connection", "Upgrade")
                .header("Upgrade", "websocket")
                .header("Sec-WebSocket-Protocol", "graphql-transport-ws")
                .header("Sec-WebSocket-Key", generate_key())
                .header("Sec-WebSocket-Version", 13);
            let headers = request_builder.headers_mut().unwrap();
            headers.insert(
                http::header::AUTHORIZATION,
                format!("Bearer {}", access_token)
                    .parse()
                    .expect("Access token parse."),
            );
            headers.insert(
                http::header::USER_AGENT,
                format!("tibberator/0.1.0 com.tibber/1.8.3")
                    .parse()
                    .expect("User agent parse."),
            );
            request_builder
        }

        /// Gets all Viewer data
        ///
        /// cf. the tibber schema. Needs an `access_token` configured in `config` to fetch data.
        ///
        /// # Examples
        ///
        async fn get_viewer(
            config: &AccessConfig,
        ) -> Result<graphql_client::Response<<Viewer as GraphQLQuery>::ResponseData>, reqwest::Error>
        {
            let variables = viewer::Variables {};
            fetch_data::<Viewer>(config, variables).await
        }

        /// Retrieves the WebSocket URL to connect to the Tibber service using the provided `access_token`.
        ///
        /// ## Errors
        /// - Returns an error if the WebSocket URL could not be retrieved.
        ///
        async fn fetch_subscription_url(
            config: &AccessConfig,
        ) -> Result<String, Box<dyn std::error::Error>> {
            let viewer_response = get_viewer(config).await?;

            match handle_response_error(viewer_response.errors) {
                Some(error) => {
                    return Err(error);
                }
                _ => {}
            }

            match viewer_response.data {
                Some(data) => {
                    let url = data.viewer.websocket_subscription_url;
                    match url {
                        Some(u) => Ok(u),
                        None => Err(Box::new(std::io::Error::new(
                            std::io::ErrorKind::NotFound,
                            "no websocket url found",
                        ))),
                    }
                }
                None => Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "no data found in viewer",
                ))),
            }
        }

        /// Retrieves all `Viewer` data from the Tibber schema.
        ///
        /// Requires an `access_token` configured in the provided `config` to fetch data.
        ///
        /// ## Parameters
        /// - `config`: An `AccessConfig` containing configuration details (e.g., access token).
        ///
        /// ## Returns
        /// A `Result` containing either:
        /// - A `graphql_client::Response` with the response data for the `Viewer` query.
        /// - An error of type `reqwest::Error` if there are issues during data retrieval.
        ///
        fn handle_response_error(
            errors: Option<Vec<graphql_client::Error>>,
        ) -> Option<Box<dyn std::error::Error>> {
            match errors {
                Some(error_list) => {
                    let mut error_string = String::new();
                    for err in error_list {
                        error_string += err.message.as_str();
                    }
                    Some(Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        error_string,
                    )))
                }
                None => None,
            }
        }

        /// Creates a websocket by connecting to the Tibber API.
        ///
        /// The `create_websocket` function establishes a WebSocket connection for a GraphQL subscription.
        /// It fetches the subscription URL using the provided `AccessConfig`, constructs the necessary request,
        /// and connects to the WebSocket server.
        ///
        /// ## Parameters
        /// - `config`: An `AccessConfig` containing configuration details (e.g., access token).
        ///
        /// ## Returns
        /// A `Result` containing either:
        /// - A tuple with a `WebSocketStream<ConnectStream>` representing the WebSocket connection
        ///   and a `Response` containing additional information.
        /// - An error of type `Box<dyn std::error::Error>` if there are issues during connection setup.
        ///
        /// # Errors
        ///
        /// The method will fail if the subscription URL could not be retrieved.
        /// The method will fail if the connection could not be established.
        ///
        async fn create_websocket(
            config: &AccessConfig,
        ) -> Result<(WebSocketStream<ConnectStream>, Response), Box<dyn std::error::Error>>
        {
            let url = fetch_subscription_url(config).await?;
            let uri = Uri::from_str(&url)?;

            let request = configure_request(&config.token, uri).body(())?;

            let result = connect_async(request).await;

            match result {
                Ok(res) => Ok(res),
                Err(err) => Err(Box::new(
                    async_tungstenite::tungstenite::error::Error::from(err),
                )),
            }
        }

        /// Retrieves all available home IDs for the provided `access_token` in the given `config`.
        ///
        /// ## Errors
        /// - Returns an error if no connection to the Tibber API could be established.
        /// - Returns an error if the response contains any errors.
        /// - Returns an error if the viewer struct contains empty data.
        ///
        /// # Example
        ///
        /// ```
        ///   use tibberator::tibber::data_handling::{AccessConfig, get_home_ids};
        ///
        /// # #[tokio::main]
        /// # async fn main() {
        ///   let config = AccessConfig::default();
        ///   let viewer_response = get_home_ids(&config).await;
        ///   assert!(viewer_response.is_ok());
        ///   assert!(!viewer_response.unwrap().is_empty())
        /// # }
        /// ```
        pub async fn get_home_ids(
            config: &AccessConfig,
        ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
            let viewer_response = get_viewer(config).await?;
            match handle_response_error(viewer_response.errors) {
                Some(error) => {
                    return Err(error);
                }
                _ => {}
            }

            match viewer_response.data {
                Some(data) => {
                    let homes = data.viewer.homes;
                    let home_ids: Vec<String> = homes
                        .into_iter()
                        .filter_map(|optional| optional)
                        .map(|home| home.id)
                        .collect();
                    Ok(home_ids)
                }
                None => Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "no data found in viewer",
                ))),
            }
        }

        /// Retrieves the current energy price information based on the provided configuration.
        ///
        /// This asynchronous function fetches data using the given `config` and constructs a `PriceInfo`
        /// struct representing the current energy price. If successful, it returns the `PriceInfo`.
        /// Otherwise, it returns an error wrapped in a `Box<dyn std::error::Error>`.
        ///
        /// ## Arguments
        ///
        /// * `config`: A reference to the `AccessConfig` containing necessary information for fetching data.
        ///
        /// ## Returns
        ///
        /// * `Result<PriceInfo, Box<dyn std::error::Error>>`: The current energy price information or an error.
        ///
        async fn get_current_energy_price(
            config: &AccessConfig,
        ) -> Result<PriceInfo, Box<dyn std::error::Error>> {
            let id = config.home_id.to_owned();
            let variables = price_current::Variables { id };
            let price_data_response = fetch_data::<PriceCurrent>(config, variables).await?;
            let price_info = price_data_response
                .data
                .ok_or(LoopEndingError::InvalidData)?
                .viewer
                .home
                .current_subscription
                .ok_or(LoopEndingError::InvalidData)?
                .price_info
                .ok_or(LoopEndingError::InvalidData)?;
            PriceInfo::new_current(price_info.current.ok_or(LoopEndingError::InvalidData)?)
                .ok_or(Box::new(LoopEndingError::InvalidData))
        }

        /// Updates or retrieves the current energy price information based on the provided configuration and existing price info.
        ///
        /// This asynchronous function checks the elapsed time since the `starts_at` timestamp in the `current_price_info`.
        /// If the elapsed time is greater than 1 hour (3600 seconds), it fetches the current energy price using the `get_current_energy_price` method.
        /// Otherwise, it returns the existing `current_price_info`.
        ///
        /// ## Arguments
        ///
        /// * `config`: A reference to the `AccessConfig` containing necessary information for fetching data.
        /// * `current_price_info`: An optional `PriceInfo` representing the existing price information (if available).
        ///
        /// ## Returns
        ///
        /// * `Result<PriceInfo, Box<dyn std::error::Error>>`: The updated or existing current energy price information, or an error.
        ///
        pub async fn update_current_energy_price_info(
            config: &AccessConfig,
            current_price_info: Option<PriceInfo>,
        ) -> Result<PriceInfo, Box<dyn std::error::Error>> {
            match current_price_info {
                Some(price_info) => {
                    let datetime_now = Utc::now();
                    let elapsed_time = datetime_now.signed_duration_since(price_info.starts_at);

                    if elapsed_time > Duration::try_seconds(3600).unwrap() {
                        get_current_energy_price(config).await
                    } else {
                        Ok(price_info)
                    }
                }
                None => get_current_energy_price(config).await,
            }
        }

        pub type LiveMeasurementSubscription = Subscription<StreamingOperation<LiveMeasurement>>;

        /// Creates the websocket subscription in order to retrieve live data from Tibber.
        ///
        /// The `create_subscription` function establishes a WebSocket connection for a GraphQL subscription request.
        /// It takes an access token, GraphQL query variables (specific to `LiveMeasurement`), and a WebSocket stream.
        /// Upon successful connection, it sends an initialization payload with the access token.
        /// It then subscribes to a GraphQL subscription operation for `LiveMeasurement` data.
        ///
        /// ## Parameters
        /// - `access_token`: A string representing the access token used for authentication.
        /// - `variables`: A data type containing the variables for the GraphQL query (specific to `LiveMeasurement`).
        /// - `websocket`: A WebSocket connection (type: `WebSocketStream<ConnectStream>`).
        ///
        /// ## Returns
        /// A `Result` containing either:
        /// - A `Subscription` object that encapsulates the data streams for the subscription.
        /// - An error of type `graphql_ws_client::Error`.
        ///
        /// ## Errors
        ///
        /// Will return an error if the connection init fails.
        ///
        async fn create_subscription(
            access_token: &str,
            variables: <LiveMeasurement as GraphQLQuery>::Variables,
            websocket: WebSocketStream<ConnectStream>,
        ) -> Result<LiveMeasurementSubscription, graphql_ws_client::Error> {
            let init_payload = serde_json::json!({"token": access_token});

            let streaming_operation = StreamingOperation::<LiveMeasurement>::new(variables);
            WSClient::build(websocket)
                .payload(init_payload)?
                .subscribe(streaming_operation)
                .await
        }

        /// Creates a live measurement websocket.
        ///
        /// A live measurement websocket is created. The websocket can be polled for LiveMeasurement
        /// data. The user `access_token` and `home_id` are needed from the `config`.
        ///
        /// # Error
        ///
        /// - Returns an error if the websocket connection fails.
        /// - Returns an error if the websocket subscription could not be established.
        /// ```
        async fn get_live_measurement(
            config: &AccessConfig,
        ) -> Result<LiveMeasurementSubscription, graphql_ws_client::Error> {
            let id = config.home_id.to_owned();
            let variables = live_measurement::Variables { id };

            let websocket = create_websocket(config).await;
            match websocket {
                Ok(value) => {
                    let (websocket, _) = value;
                    create_subscription(&config.token, variables, websocket).await
                }
                Err(error) => {
                    return Err(graphql_ws_client::Error::Unknown(
                        String::from("Websocket creation error: ") + error.to_string().as_str(),
                    ));
                }
            }
        }

        /// Establishes a connection to the Tibber Subscription websocket for live measurement data.
        ///
        /// This function attempts to create a subscription for streaming live measurement data
        /// from the Tibber service. If successful, it returns a valid `Subscription` containing
        /// the streaming operation. If an error occurs during the subscription process, an error
        /// message is printed, and the program exits with the `exitcode::PROTOCOL` status code.
        ///
        /// # Arguments
        ///
        /// * `config`: A reference to an `AccessConfig` containing the necessary configuration
        ///   parameters for connecting to the Tibber service.
        ///
        /// # Returns
        ///
        /// A `Subscription<StreamingOperation<LiveMeasurement>>`:
        /// - If the connection is established successfully, returns a valid subscription for
        ///   live measurement data.
        /// - If an error occurs during the subscription process, the program exits.
        ///
        /// # Examples
        ///
        /// ```
        ///   use tibberator::tibber::data_handling::{AccessConfig, connect_live_measurement};
        ///
        /// # #[tokio::main]
        /// # async fn main() {
        ///   let config = AccessConfig::default();
        ///   let subscription = connect_live_measurement(&config).await;
        ///   assert!(subscription.stop().await.is_ok());
        /// # }
        /// ```
        pub async fn connect_live_measurement(
            config: &AccessConfig,
        ) -> LiveMeasurementSubscription {
            let subscription = get_live_measurement(&config).await;

            match subscription {
                Ok(result) => {
                    println!("Connection established");
                    result
                }
                Err(error) => {
                    println!("{:?}", error);
                    std::process::exit(exitcode::PROTOCOL);
                }
            }
        }

        #[cfg(test)]
        mod tests {
            use super::*;
            use serial_test::serial;
            use tokio::time::timeout;

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

                let mut subscription = connect_live_measurement(&config).await;

                for _ in 1..=5 {
                    let result =
                        timeout(std::time::Duration::from_secs(90), subscription.next()).await;
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
        }
    }

    mod output {
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

        #[derive(Debug, Serialize, Deserialize, PartialEq)]
        pub enum OutputType {
            Full,
            Silent,
        }

        #[derive(Debug, Serialize, Deserialize, PartialEq)]
        pub enum TaxStyle {
            Price,
            Percent,
            None,
        }

        #[derive(Debug, Serialize, Deserialize)]
        pub struct OutputConfig {
            output_type: OutputType,
            tax_style: TaxStyle,
        }

        impl Default for OutputConfig {
            fn default() -> Self {
                OutputConfig {
                    output_type: OutputType::Full,
                    tax_style: TaxStyle::Price,
                }
            }
        }

        impl OutputConfig {
            pub fn is_silent(&self) -> bool {
                self.output_type == OutputType::Silent
            }

            pub fn new(output_type: OutputType) -> Self {
                OutputConfig {
                    output_type,
                    tax_style: TaxStyle::None,
                }
            }

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
                    format!(" Tax: {:.1} %", price_info.tax / price_info.total * 100.)
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
    }

    #[derive(Debug, Serialize, Deserialize)]
    pub struct Config {
        pub access: AccessConfig,
        pub output: OutputConfig,
    }

    impl Default for Config {
        fn default() -> Self {
            Config {
                access: AccessConfig::default(),
                output: OutputConfig::default(),
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
    /// ```
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
    /// ```
    ///   use std::sync::mpsc::{channel, Receiver};
    ///   use tibberator::tibber::{
    ///                            Config, loop_for_data,
    ///                            data_handling::{AccessConfig, connect_live_measurement},
    ///                           };
    ///   use tokio::time;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    ///   let config = Config::default();
    ///   let mut subscription = connect_live_measurement(&config.access).await;
    ///   let (sender, receiver) = channel();
    ///   tokio::spawn(async move {
    ///     std::thread::sleep(time::Duration::from_secs(3));
    ///     sender.send(true).unwrap();
    ///   });
    ///   let result = loop_for_data(&config, &mut subscription, &receiver).await;
    ///   assert!(result.is_ok());
    /// # }
    /// ```
    pub async fn loop_for_data(
        config: &Config,
        subscription: &mut Subscription<StreamingOperation<LiveMeasurement>>,
        receiver: &Receiver<bool>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let last_value_received = Rc::new(Cell::new(Instant::now()));
        let stop_fun = future::poll_fn(|_cx| {
            if check_user_shutdown(&receiver) == true {
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

                        if !config.output.is_silent() {
                            print_screen(
                                &config.output.get_tax_style(),
                                current_state,
                                &current_price_info,
                            );
                        }

                        current_price_info = update_current_energy_price_info(
                            &config.access,
                            Some(current_price_info),
                        )
                        .await?;
                    }
                    None => {
                        return Err(Box::new(LoopEndingError::InvalidData));
                    }
                },
                Ok(None) => break,
                Err(_) => {
                    return Err(Box::new(LoopEndingError::Reconnect));
                }
            }
        }
        match stream.take_result() {
            Some(LoopEndingError::Shutdown) => Ok(()),
            Some(LoopEndingError::Reconnect) => Err(Box::new(LoopEndingError::Reconnect)),
            _ => Err(Box::new(LoopEndingError::InvalidData)),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use chrono::{DateTime, Duration, FixedOffset};
        use serial_test::serial;
        use std::sync::mpsc::channel;
        use tokio::time::timeout;

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
            use tokio::time;

            let config = Config {
                access: AccessConfig::default(),
                output: OutputConfig::new(OutputType::Silent),
            };
            let mut subscription = Box::new(connect_live_measurement(&config.access).await);

            let (sender, receiver) = channel();
            let result = loop_for_data(&config, subscription.as_mut(), &receiver);
            tokio::time::sleep(time::Duration::from_secs(10)).await;
            sender.send(true).unwrap();
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
                output: OutputConfig::new(OutputType::Silent),
            };
            config.access.home_id.pop();
            let mut subscription = Box::new(connect_live_measurement(&config.access).await);

            let (_sender, receiver) = channel();
            let result = timeout(
                std::time::Duration::from_secs(10),
                loop_for_data(&config, subscription.as_mut(), &receiver),
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
                output: OutputConfig::new(OutputType::Silent),
            };
            config.access.reconnect_timeout = 0;
            let mut subscription = Box::new(connect_live_measurement(&config.access).await);

            let (_sender, receiver) = channel();
            let result = loop_for_data(&config, subscription.as_mut(), &receiver).await;
            assert!(result.as_ref().is_err());
            let error = result.err().unwrap();
            let error_type = error.downcast::<LoopEndingError>();
            assert!(error_type.is_ok());
            assert_eq!(*error_type.unwrap(), LoopEndingError::Reconnect);
            subscription.stop().await.unwrap();
        }
    }
}
