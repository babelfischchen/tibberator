//! Module for easy access to the Tibber API
//!
//! This module contains various helper methods to connect to the Tibber API (see https://developer.tibber.com).
//! You need an access token in order to use the API.
pub mod tibber {
    use async_tungstenite::{
        async_std::{connect_async, ConnectStream},
        tungstenite::handshake::client::{generate_key, Response},
        WebSocketStream,
    };
    use graphql_client::{reqwest::post_graphql, GraphQLQuery};
    use graphql_ws_client::{graphql::StreamingOperation, Client as WSClient, Subscription};
    use http::{request::Builder, Request, Uri};
    use reqwest::{
        header::{HeaderMap, AUTHORIZATION},
        Client,
    };
    use serde::{Deserialize, Serialize};
    use std::str::FromStr;

    #[derive(Debug, Serialize, Deserialize)]
    pub struct AccessConfig {
        pub token: String,
        url: String,
        pub home_id: String,
        pub reconnect_timeout: i32,
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
    ) -> Result<(WebSocketStream<ConnectStream>, Response), Box<dyn std::error::Error>> {
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
    ///   use tibberator::tibber;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    ///   let config = tibber::AccessConfig::default();
    ///   let home_response = tibber::fetch_home_data(&config).await.is_ok();
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

    /// Gets all Viewer data
    ///
    /// cf. the tibber schema. Needs an `access_token` configured in `config` to fetch data.
    ///
    /// # Examples
    ///
    /// ```
    ///   use tibberator::tibber;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    ///   let config = tibber::AccessConfig::default();
    ///   let viewer_response = tibber::get_viewer(&config).await.is_ok();
    ///   assert!(viewer_response);
    /// # }
    /// ```
    pub async fn get_viewer(
        config: &AccessConfig,
    ) -> Result<graphql_client::Response<<Viewer as GraphQLQuery>::ResponseData>, reqwest::Error>
    {
        let variables = viewer::Variables {};
        fetch_data::<Viewer>(config, variables).await
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
    ///   use tibberator::tibber;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    ///   let config = tibber::AccessConfig::default();
    ///   let viewer_response = tibber::get_home_ids(&config).await;
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

    /// Retrieves the WebSocket URL to connect to the Tibber service using the provided `access_token`.
    ///
    /// ## Errors
    /// - Returns an error if the WebSocket URL could not be retrieved.
    ///
    /// ## Example
    ///
    /// ```
    ///   use tibberator::tibber;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    ///   let config = tibber::AccessConfig::default();
    ///   let subscription_url = tibber::fetch_subscription_url(&config).await.is_ok();
    ///   assert!(subscription_url);
    /// # }
    /// ```
    pub async fn fetch_subscription_url(
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
    ) -> Result<Subscription<StreamingOperation<LiveMeasurement>>, graphql_ws_client::Error> {
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
    ///
    /// # Examples
    ///
    /// ```
    ///   use tibberator::tibber;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    ///   let config = tibber::AccessConfig::default();
    ///   let subscription = tibber::get_live_measurement(&config).await;
    ///   assert!(subscription.is_ok());
    ///   assert!(subscription.unwrap().stop().await.is_ok());
    /// # }
    /// ```
    pub async fn get_live_measurement(
        config: &AccessConfig,
    ) -> Result<Subscription<StreamingOperation<LiveMeasurement>>, graphql_ws_client::Error> {
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

    #[cfg(test)]
    mod tests {
        use super::*;

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
        async fn test_create_websocket() {
            let config = AccessConfig::default();

            let result = create_websocket(&config).await;
            assert!(result.is_ok());
            let (mut test_instance, _) = result.unwrap();
            assert!(test_instance.close(None).await.is_ok());
        }

        #[tokio::test]
        async fn test_get_live_measurement() {
            use futures::stream::StreamExt;
            let config = AccessConfig::default();

            let mut subscription = get_live_measurement(&config).await.unwrap();

            for _ in 1..=5 {
                let item = subscription.next().await;
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
