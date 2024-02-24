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
        token: String,
        url: String,
        home_id: String,
    }

    impl Default for AccessConfig {
        fn default() -> Self {
            AccessConfig {
                token: "5K4MVS-OjfWhK_4yrjOlFe1F6kJXPVf7eQYggo8ebAE".to_string(),
                url: "https://api.tibber.com/v1-beta/gql".to_string(),
                home_id: "96a14971-525a-4420-aae9-e5aedaa129ff".to_string(),
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

    /// Creates a reqwest client using the `access token`.
    ///
    /// # Errors
    /// This method will return an error if the client could not be created.
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
    /// The method uses `access_token` and the tibber `uri` to create a valid http request.
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
            format!("tibberator/0.0.1 com.tibber/1.8.3")
                .parse()
                .expect("User agent parse."),
        );
        request_builder
    }

    /// Creates a websocket by connecting to the Tibber API.
    ///
    /// The method fetches the subscription URL to use from the Tibber API itself. Then
    /// it configures the http-request and asynchronously connects the websocket.
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

    /// Handles the errors in a graphql_client response `errors` structure.
    ///
    /// Returns `None` if there were no errors. Returns an error if the
    /// structure contained any errors.
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

    /// Fetches all available home ids for the provided `access_token` in `config`.
    ///
    /// # Errors
    ///
    /// Returns an error if no connection to the Tibber API could be established.
    /// Returns an error if the response contained any errors.
    /// Returns an error if the viewer struct contains empty data.
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

    /// Gets the websocket url to connect to the tibber service using the `access_token`.
    ///
    /// # Errors
    ///
    /// Will return an error if the websocket URL could not be retrieved
    ///
    /// # Examples
    ///
    /// ```
    ///   use tibberator::tibber;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    ///   let demo_token = "5K4MVS-OjfWhK_4yrjOlFe1F6kJXPVf7eQYggo8ebAE";
    ///   let subscription_url = tibber::fetch_subscription_url(demo_token).await.is_ok();
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
    /// The method needs the Tibber `access_token` as well as the GraphQL `variables`
    /// for the LiveMeasurement. Also the open `websocket` must be provided.
    ///
    /// # Errors
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
