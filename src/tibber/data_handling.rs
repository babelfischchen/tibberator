/// This module handles connectivity and data for the tibberator application.
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
use log::{error, info, warn};
use reqwest::{
    header::{HeaderMap, AUTHORIZATION},
    Client,
};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use thiserror::Error;
use tokio::time::timeout;

/// `Home` is a struct that represents a GraphQL query for the `home` endpoint.
#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "tibber/schema.json",
    query_path = "tibber/home.graphql",
    respone_derives = "Debug"
)]
pub struct Home;

/// `Viewer` is a struct that represents a GraphQL query for the `viewer` endpoint.
#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "tibber/schema.json",
    query_path = "tibber/view.graphql",
    respone_derives = "Debug"
)]
pub struct Viewer;

/// `LiveMeasurement` is a struct that represents a GraphQL query for the `live_measurement` endpoint.
#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "tibber/schema.json",
    query_path = "tibber/livemeasurement.graphql",
    respone_derives = "Debug"
)]
pub struct LiveMeasurement;

/// `PriceCurrent` is a struct that represents a GraphQL query for the `price_current` endpoint.
#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "tibber/schema.json",
    query_path = "tibber/price_current.graphql",
    respone_derives = "Debug"
)]
struct PriceCurrent;

/// `LoopEndingError` is an enum that represents the different types of errors that can occur when a loop ends.
/// It can be one of the following: `Shutdown`, `Reconnect`, or `InvalidData`.
#[derive(Debug, Error, PartialEq)]
pub enum LoopEndingError {
    #[error("Shutdown requested")]
    Shutdown,
    #[error("Connection timed out. Reconnection necessary.")]
    Reconnect,
    #[error("Invalid or no data received.")]
    InvalidData,
    #[error("Connection timed out while waiting for data.")]
    ConnectionTimeout,
}

/// `AccessConfig` is a struct that represents the configuration for accessing the Tibber service.
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

/// `PriceLevel` is an enum that represents the different levels of price.
/// It can be one of the following: `VeryCheap`, `Cheap`, `Normal`, `Expensive`, `VeryExpensive`, `Other(String)`, or `None`.
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

/// The `Default` implementation for `PriceLevel` provides a default instance of `PriceLevel` as `None`.
impl Default for PriceLevel {
    fn default() -> Self {
        PriceLevel::None
    }
}

/// `PriceInfo` is a struct that represents the information about the tibber energy price at a certain time.
/// It contains the following fields: `total`, `energy`, `tax`, `starts_at`, `currency`, and `level`.
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
    /// The `new_current` method for `PriceInfo` provides a way to create a new instance of `PriceInfo` from a given `price_info` representing the current energy prices.
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

/// Asynchronously fetches data from a GraphQL endpoint.
///
/// This function takes in a configuration object and a set of variables, and returns a `Result` containing either a `Response` object with the response data or an `Error`.
///
/// ## Arguments
///
/// * `config`: A reference to an `AccessConfig` object that contains the access configuration for the GraphQL endpoint.
/// * `variables`: A set of variables for the GraphQL query. The type of these variables is determined by the `GraphQLQuery` trait.
///
/// ## Returns
///
/// * `Result<graphql_client::Response<<T as GraphQLQuery>::ResponseData>, reqwest::Error>`: A `Result` object that contains either a `Response` object with the response data or an `Error`.
///
/// ## Type Parameters
///
/// * `T`: The type that implements the `GraphQLQuery` trait.
///
/// ## Errors
///
/// This function will return an `Error` if the request fails.
///
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
///   use tibberator::tibber::{AccessConfig, fetch_home_data};
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
) -> Result<graphql_client::Response<<Home as GraphQLQuery>::ResponseData>, reqwest::Error> {
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
async fn get_viewer(
    config: &AccessConfig,
) -> Result<graphql_client::Response<<Viewer as GraphQLQuery>::ResponseData>, reqwest::Error> {
    let variables = viewer::Variables {};
    fetch_data::<Viewer>(config, variables).await
}

/// Asynchronously fetches the subscription URL from a GraphQL endpoint.
///
/// This function takes in a configuration object and returns a `Result` containing either a `String` with the subscription URL or an `Error`.
///
/// ## Arguments
///
/// * `config`: A reference to an `AccessConfig` object that contains the access configuration for the GraphQL endpoint.
///
/// ## Returns
///
/// * `Result<String, Box<dyn std::error::Error>>`: A `Result` object that contains either a `String` with the subscription URL or an `Error`.
///
/// ## Errors
///
/// This function will return an `Error` if the request fails, no websocket URL is found, or no data is found in the viewer.
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

/// Handles the errors returned from a GraphQL response.
///
/// This function takes in an `Option` containing a list of GraphQL errors and returns an `Option` containing a boxed `Error` if there are any errors, or `None` if there are no errors.
///
/// ## Arguments
///
/// * `errors`: An `Option` containing a vector of GraphQL errors.
///
/// ## Returns
///
/// * `Option<Box<dyn std::error::Error>>`: An `Option` containing a boxed `Error` if there are any errors, or `None` if there are no errors.
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
///   use tibberator::tibber::{AccessConfig, get_home_ids};
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
    info!(target: "tibberator.price", "Fetching current energy price.");

    let id = config.home_id.to_owned();
    let variables = price_current::Variables { id };
    let price_data_response = timeout(
        tokio::time::Duration::from_secs(10),
        fetch_data::<PriceCurrent>(config, variables),
    )
    .await?;

    let price_info = price_data_response?
        .data
        .ok_or(LoopEndingError::InvalidData)?
        .viewer
        .home
        .current_subscription
        .and_then(|info| info.price_info)
        .ok_or(LoopEndingError::InvalidData)?;

    info!(target: "tibberator.price", "Received price data");
    if let Some(current_price) = price_info.current.as_ref().or(None) {
        info!(target: "tibberator.price", "Current price: {:?}", current_price.total);
    } else {
        warn!(target: "tibberator.price", "Current price not available in data");
    }

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

pub type LiveMeasurementOperation = StreamingOperation<LiveMeasurement>;
pub type LiveMeasurementSubscription = Subscription<LiveMeasurementOperation>;

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
///
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
///   use tibberator::tibber::{AccessConfig, connect_live_measurement};
///
/// # #[tokio::main]
/// # async fn main() {
///   let config = AccessConfig::default();
///   let subscription = connect_live_measurement(&config).await;
///   assert!(subscription.stop().await.is_ok());
/// # }
/// ```
pub async fn connect_live_measurement(config: &AccessConfig) -> LiveMeasurementSubscription {
    let subscription = get_live_measurement(&config).await;

    match subscription {
        Ok(result) => {
            info!(target: "tibberator.connection", "Successfully connected to Tibber service.");
            println!("Connection established");
            result
        }
        Err(error) => {
            error!(target: "tibberator.connection", "Failed to connect to Tibber service: {:?}", error);
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
}
