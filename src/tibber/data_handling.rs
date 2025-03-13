/// This module handles connectivity and data for the tibberator application.
use async_tungstenite::{
    async_std::{connect_async, ConnectStream},
    tungstenite::handshake::client::{generate_key, Response},
    WebSocketStream,
};
use chrono::{DateTime, Datelike, Duration, FixedOffset, Local, TimeZone, Utc};
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
    response_derives = "Debug"
)]
pub struct Home;

/// `Viewer` is a struct that represents a GraphQL query for the `viewer` endpoint.
#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "tibber/schema.json",
    query_path = "tibber/view.graphql",
    response_derives = "Debug"
)]
pub struct Viewer;

/// `LiveMeasurement` is a struct that represents a GraphQL query for the `live_measurement` endpoint.
#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "tibber/schema.json",
    query_path = "tibber/livemeasurement.graphql",
    response_derives = "Debug"
)]
pub struct LiveMeasurement;

/// `PriceCurrent` is a struct that represents a GraphQL query for the `price_current` endpoint.
#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "tibber/schema.json",
    query_path = "tibber/price_current.graphql",
    response_derives = "Debug"
)]
struct PriceCurrent;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "tibber/schema.json",
    query_path = "tibber/price_hourly.graphql",
    response_derives = "Debug"
)]
struct PriceHourly;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "tibber/schema.json",
    query_path = "tibber/consumption_hourly.graphql",
    response_derives = "Debug"
)]
struct ConsumptionHourly;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "tibber/schema.json",
    query_path = "tibber/consumption_hourly_page_info.graphql",
    response_derives = "Debug"
)]
struct ConsumptionHourlyPageInfo;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "tibber/schema.json",
    query_path = "tibber/fee_estimation.graphql",
    response_derives = "Debug"
)]
struct FeeEstimation;

/// Trait to define the common fields required for parsing price info.
trait PriceInfoFields {
    fn total(&self) -> Option<f64>;
    fn energy(&self) -> Option<f64>;
    fn tax(&self) -> Option<f64>;
    fn starts_at(&self) -> Option<String>;
    fn level(&self) -> PriceLevel;
    fn currency(&self) -> String;
}

impl PriceInfoFields for price_current::PriceCurrentViewerHomeCurrentSubscriptionPriceInfoCurrent {
    fn total(&self) -> Option<f64> {
        self.total
    }
    fn energy(&self) -> Option<f64> {
        self.energy
    }
    fn tax(&self) -> Option<f64> {
        self.tax
    }
    fn starts_at(&self) -> Option<String> {
        self.starts_at.clone()
    }
    fn level(&self) -> PriceLevel {
        match &self.level {
            Some(price_current::PriceLevel::VERY_CHEAP) => PriceLevel::VeryCheap,
            Some(price_current::PriceLevel::CHEAP) => PriceLevel::Cheap,
            Some(price_current::PriceLevel::NORMAL) => PriceLevel::Normal,
            Some(price_current::PriceLevel::EXPENSIVE) => PriceLevel::Expensive,
            Some(price_current::PriceLevel::VERY_EXPENSIVE) => PriceLevel::VeryExpensive,
            Some(price_current::PriceLevel::Other(string)) => PriceLevel::Other(string.clone()),
            _ => PriceLevel::None,
        }
    }
    fn currency(&self) -> String {
        self.currency.clone()
    }
}

impl PriceInfoFields for price_hourly::PriceHourlyViewerHomeCurrentSubscriptionPriceInfoToday {
    fn total(&self) -> Option<f64> {
        self.total
    }
    fn energy(&self) -> Option<f64> {
        self.energy
    }
    fn tax(&self) -> Option<f64> {
        self.tax
    }
    fn starts_at(&self) -> Option<String> {
        self.starts_at.clone()
    }
    fn level(&self) -> PriceLevel {
        match &self.level {
            Some(price_hourly::PriceLevel::VERY_CHEAP) => PriceLevel::VeryCheap,
            Some(price_hourly::PriceLevel::CHEAP) => PriceLevel::Cheap,
            Some(price_hourly::PriceLevel::NORMAL) => PriceLevel::Normal,
            Some(price_hourly::PriceLevel::EXPENSIVE) => PriceLevel::Expensive,
            Some(price_hourly::PriceLevel::VERY_EXPENSIVE) => PriceLevel::VeryExpensive,
            Some(price_hourly::PriceLevel::Other(string)) => PriceLevel::Other(string.clone()),
            _ => PriceLevel::None,
        }
    }
    fn currency(&self) -> String {
        self.currency.clone()
    }
}

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
#[derive(Debug, Serialize, Deserialize, Clone)]
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
    fn parse_price_info(price_info: &impl PriceInfoFields) -> Option<Self> {
        let total = price_info.total()?;
        let energy = price_info.energy()?;
        let tax = price_info.tax()?;
        let currency = price_info.currency();
        let starts_at = chrono::DateTime::parse_from_rfc3339(
            price_info.starts_at().ok_or("No timestamp").ok()?.as_str(),
        )
        .ok()?;
        let price_level = price_info.level();

        Some(PriceInfo {
            total,
            energy,
            tax,
            currency,
            starts_at,
            level: price_level,
        })
    }

    /// The `new_current` method for `PriceInfo` provides a way to create a new instance of `PriceInfo` from a given `price_info` representing the current energy prices.
    fn new_current(
        price_info: price_current::PriceCurrentViewerHomeCurrentSubscriptionPriceInfoCurrent,
    ) -> Option<Self> {
        Self::parse_price_info(&price_info)
    }

    fn new_hourly(
        all_price_info: price_hourly::PriceHourlyViewerHomeCurrentSubscriptionPriceInfo,
    ) -> Option<Vec<PriceInfo>> {
        let mut price_infos = Vec::new();

        for hour in all_price_info.today {
            if let Some(current_hour) = hour {
                if let Some(price_info) = Self::parse_price_info(&current_hour) {
                    price_infos.push(price_info);
                }
            }
        }

        Some(price_infos)
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

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConsumptionPage {
    pub start_cursor: String,
    pub has_previous_page: bool,
    pub count: usize,
    pub total_cost: f64,
    pub total_consumption: f64,
    pub currency: String,
}

impl ConsumptionPage {
    pub fn create_from(
        consumption_page: &consumption_hourly_page_info::ConsumptionHourlyPageInfoViewerHomeConsumptionPageInfo,
    ) -> Option<ConsumptionPage> {
        let start_cursor = String::from(consumption_page.start_cursor.clone()?);
        let has_previous_page = consumption_page.has_previous_page?;
        let count = consumption_page.count? as usize;
        let total_cost = consumption_page.total_cost?;
        let total_consumption = consumption_page.total_consumption?;
        let currency = String::from(consumption_page.currency.clone()?);

        Some(ConsumptionPage {
            start_cursor,
            has_previous_page,
            count,
            total_cost,
            total_consumption,
            currency,
        })
    }
}

/// `ConsumptionNode` is a struct that represents a single consumption data point.
#[derive(Debug, Clone)]
pub struct ConsumptionNode {
    pub from: DateTime<FixedOffset>,
    pub to: DateTime<FixedOffset>,
    pub consumption: f64,
    pub cost: f64,
}

impl ConsumptionNode {
    /// Parses a single consumption data point and returns a `ConsumptionNode`.
    fn parse_consumption_node(
        consumption_data: &consumption_hourly::ConsumptionHourlyViewerHomeConsumptionNodes,
    ) -> Option<Self> {
        let from = chrono::DateTime::parse_from_rfc3339(consumption_data.from.as_str()).ok()?;
        let to = chrono::DateTime::parse_from_rfc3339(consumption_data.to.as_str()).ok()?;
        let consumption = consumption_data.consumption.unwrap_or(0.0);
        let cost = consumption_data.cost.unwrap_or(0.0);

        Some(ConsumptionNode {
            from,
            to,
            consumption,
            cost,
        })
    }

    /// The `new_nodes` method for `ConsumptionNode` provides a way to create a vector of `ConsumptionNode` instances from a given list of consumption data.
    fn new_nodes(
        consumption_list: consumption_hourly::ConsumptionHourlyViewerHomeConsumption,
    ) -> Option<Vec<Self>> {
        let mut nodes = Vec::new();

        for consumption_data in consumption_list.nodes?.into_iter() {
            if let Some(node) = consumption_data {
                if let Some(parsed_node) = Self::parse_consumption_node(&node) {
                    nodes.push(parsed_node);
                }
            }
        }

        Some(nodes)
    }

    /// The `new_nodes_today` method for `ConsumptionNode` provides a way to create a vector of `ConsumptionNode` instances from a given list of consumption data,
    /// filtering only those nodes that belong to the current day and filling in missing hours with 0 values for consumption and cost.
    fn new_nodes_today(
        consumption_list: consumption_hourly::ConsumptionHourlyViewerHomeConsumption,
    ) -> Option<Vec<Self>> {
        let nodes = Self::new_nodes(consumption_list)?;

        let today = Utc::now().date_naive();
        let timezone = Local;
        let mut hourly_nodes = Vec::new();

        // Generate hourly nodes for the current day
        for hour in 0..24 {
            let from = today
                .and_hms_opt(hour, 0, 0)?
                .and_local_timezone(timezone.offset_from_utc_date(&today))
                .single()?;
            let to = from + Duration::hours(1);

            // Check if there is an existing node for this hour
            if let Some(existing_node) =
                nodes.iter().find(|node| node.from == from && node.to == to)
            {
                hourly_nodes.push(existing_node.clone());
            } else {
                // If no existing node, create a new one with 0 consumption and cost
                hourly_nodes.push(ConsumptionNode {
                    from,
                    to,
                    consumption: 0.0,
                    cost: 0.0,
                });
            }
        }

        Some(hourly_nodes)
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

/// Retrieves today's hourly energy price information based on the provided configuration.
///
/// This asynchronous function fetches data using the given `config` and constructs a `Vec<PriceInfo>`
/// struct representing today's hourly energy prices. If successful, it returns the `Vec<PriceInfo>`.
/// Otherwise, it returns an error wrapped in a `Box<dyn std::error::Error>`.
///
/// ## Arguments
///
/// * `config`: A reference to the `AccessConfig` containing necessary information for fetching data.
///
/// ## Returns
///
/// * `Result<Vec<PriceInfo>, Box<dyn std::error::Error>>`: Today's hourly energy prices or an error.
///
pub async fn get_todays_energy_prices(
    config: &AccessConfig,
) -> Result<Vec<PriceInfo>, Box<dyn std::error::Error>> {
    info!(target: "tibberator.price", "Fetching today's energy prices.");

    let id = config.home_id.to_owned();
    let variables = price_hourly::Variables { id };
    let price_data_response = timeout(
        tokio::time::Duration::from_secs(10),
        fetch_data::<PriceHourly>(config, variables),
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

    info!(target: "tibberator.price", "Received hourly price data");

    PriceInfo::new_hourly(price_info).ok_or(Box::new(LoopEndingError::InvalidData))
}

/// Retrieves today's hourly energy consumption information based on the provided configuration.
///
/// This asynchronous function fetches data using the given `config` and constructs a `Vec<ConsumptionNode>`
/// struct representing today's hourly energy consumption. If successful, it returns the `Vec<ConsumptionNode>`.
/// Otherwise, it returns an error wrapped in a `Box<dyn std::error::Error>`.
///
/// ## Arguments
///
/// * `config`: A reference to the `AccessConfig` containing necessary information for fetching data.
///
/// ## Returns
///
/// * `Result<Vec<ConsumptionNode>, Box<dyn std::error::Error>>`: Today's hourly energy consumption or an error.
///
pub async fn get_todays_energy_consumption(
    config: &AccessConfig,
) -> Result<Vec<ConsumptionNode>, Box<dyn std::error::Error>> {
    info!(target: "tibberator.consumption", "Fetching today's energy consumption.");

    let id = config.home_id.to_owned();
    let variables = consumption_hourly::Variables { id };
    let consumption_data_response = timeout(
        tokio::time::Duration::from_secs(10),
        fetch_data::<ConsumptionHourly>(config, variables),
    )
    .await?;

    let consumption_data = consumption_data_response?
        .data
        .ok_or(LoopEndingError::InvalidData)?
        .viewer
        .home
        .consumption
        .ok_or(LoopEndingError::InvalidData)?;

    info!(target: "tibberator.consumption", "Received hourly consumption data");

    ConsumptionNode::new_nodes_today(consumption_data).ok_or(Box::new(LoopEndingError::InvalidData))
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

async fn get_consumption_page(
    config: &AccessConfig,
    cursor: &String,
) -> Result<(ConsumptionPage, DateTime<FixedOffset>), Box<dyn std::error::Error>> {
    info!(target: "tibberator.consumption", "Fetching consumption page: {:?}", cursor);

    let id = config.home_id.to_owned();
    let variables = consumption_hourly_page_info::Variables {
        id,
        cursor: Some(cursor.to_owned()),
    };
    let consumption_page_response = timeout(
        tokio::time::Duration::from_secs(10),
        fetch_data::<ConsumptionHourlyPageInfo>(config, variables),
    )
    .await?;

    let consumption_page_data = consumption_page_response?
        .data
        .ok_or(LoopEndingError::InvalidData)?
        .viewer
        .home
        .consumption
        .ok_or(LoopEndingError::InvalidData)?;

    let consumption_page = consumption_page_data.page_info;
    let edges = consumption_page_data.edges;

    info!(target: "tibberator.consumption", "Received consumption page");

    let page = ConsumptionPage::create_from(&consumption_page)
        .ok_or(Box::new(LoopEndingError::InvalidData))?;
    let time = DateTime::parse_from_rfc3339(
        edges
            .ok_or(LoopEndingError::InvalidData)?
            .first()
            .ok_or(LoopEndingError::InvalidData)?
            .as_ref()
            .ok_or(LoopEndingError::InvalidData)?
            .node
            .to
            .as_str(),
    )?;
    Ok((page, time))
}

/// Estimates the daily fees based on the provided configuration.
///
/// Returns:
///   - Ok(Some(fee)): Successfully calculated daily fee
///   - Ok(None): No data available to calculate fees
///   - Err: An error occurred during calculation
pub async fn get_last_consumption_pages(
    config: &AccessConfig,
    n: usize,
) -> Result<(Vec<ConsumptionPage>, DateTime<FixedOffset>), Box<dyn std::error::Error>> {
    info!(target: "tibberator.consumption", "Retrieving last {} consumption pages", n);

    let (mut page, time) = get_consumption_page(&config, &String::from("")).await?;

    let mut pages = Vec::new();

    for _i in 0..n {
        let start_cursor = page.start_cursor.clone();
        pages.push(page);

        (page, _) = get_consumption_page(&config, &start_cursor).await?;
    }

    info!(target: "tibberator.consumption", "Received {} consumption pages", n);

    pages.reverse();
    Ok((pages, time))
}

pub async fn estimate_daily_fees(
    config: &AccessConfig,
) -> Result<Option<f64>, Box<dyn std::error::Error>> {
    info!(target: "tibberator.price", "Estimating daily fees");
    let id = config.home_id.to_owned();
    let variables = fee_estimation::Variables { id };

    let fee_estimation_response = timeout(
        tokio::time::Duration::from_secs(10),
        fetch_data::<FeeEstimation>(config, variables),
    )
    .await?;

    let fee_estimation_data = fee_estimation_response?
        .data
        .ok_or(LoopEndingError::InvalidData)?
        .viewer
        .home
        .consumption
        .ok_or(LoopEndingError::InvalidData)?;

    let price_nodes = fee_estimation_data
        .nodes
        .ok_or(LoopEndingError::InvalidData)?;

    if price_nodes.is_empty() {
        info!(target: "tibberator.price", "No price data available");
        return Ok(None);
    }

    if price_nodes.len() != 1 {
        warn!(target: "tibberator.price", "Expect 1 price node to be found, found {}", price_nodes.len());
        return Err(Box::new(LoopEndingError::InvalidData));
    }

    let price_first_node = price_nodes[0]
        .as_ref()
        .ok_or(LoopEndingError::InvalidData)?;
    let price_excluding_fees = price_first_node.cost.ok_or(LoopEndingError::InvalidData)?;

    let now = Local::now();
    let days_last_month = chrono::NaiveDate::from_ymd_opt(now.year(), now.month(), 1)
        .unwrap()
        .pred_opt()
        .unwrap()
        .day() as f64;

    match fee_estimation_data.page_info.total_cost {
        Some(cost) => {
            let daily_fee = (cost - price_excluding_fees) / days_last_month as f64;
            info!(target: "tibberator.price", "Estimated daily fee: {} {}", cost, fee_estimation_data.page_info.currency.unwrap_or_default());
            Ok(Some(daily_fee))
        }
        None => {
            info!(target: "tibberator.price", "Error calculating daily fee: Total cost not provided");
            Ok(None)
        }
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
) -> Result<LiveMeasurementSubscription, Box<dyn std::error::Error + Send + Sync>> {
    let id = config.home_id.to_owned();
    let variables = live_measurement::Variables { id };

    let websocket = create_websocket(config).await;
    match websocket {
        Ok(value) => {
            let (websocket, _) = value;
            match create_subscription(&config.token, variables, websocket).await {
                Ok(subscription) => Ok(subscription),
                Err(error) => Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    String::from("Subscription creation error: ") + error.to_string().as_str(),
                ))),
            }
        }
        Err(error) => {
            return Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                String::from("Websocket creation error: ") + error.to_string().as_str(),
            )));
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
            result
        }
        Err(error) => {
            error!(target: "tibberator.connection", "Failed to connect to Tibber service: {:?}", error);
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

    #[tokio::test]
    async fn test_get_todays_energy_consumption() {
        let config = AccessConfig::default();

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

        let result = get_consumption_page(&config, &String::from("")).await;
        assert!(result.is_ok());

        let (consumption_page, _) = result.unwrap();
        assert_eq!(consumption_page.count, 1, "Should have 1 page entries");
        assert_ne!(consumption_page.currency, "");
        assert!(consumption_page.has_previous_page);
        assert_ne!(consumption_page.start_cursor, "");
        assert!(consumption_page.total_consumption > 0.0);
        assert!(consumption_page.total_cost > 0.0);
    }

    #[tokio::test]
    async fn test_get_last_10_hours_consumption() {
        use std::collections::HashSet;

        let config = AccessConfig::default();
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
}
