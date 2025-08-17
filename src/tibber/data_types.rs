use chrono::{DateTime, Duration, FixedOffset, Local, TimeZone, Timelike, Utc};
use crossterm::style;
use graphql_client::GraphQLQuery;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use thiserror::Error;

/// `PriceCurrent` is a struct that represents a GraphQL query for the `price_current` endpoint.
#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "tibber/schema.json",
    query_path = "tibber/price_current.graphql",
    response_derives = "Debug"
)]
pub struct PriceCurrent;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "tibber/schema.json",
    query_path = "tibber/price_hourly.graphql",
    response_derives = "Debug"
)]
pub struct PriceHourly;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "tibber/schema.json",
    query_path = "tibber/consumption_hourly.graphql",
    response_derives = "Debug"
)]
pub struct Consumption;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "tibber/schema.json",
    query_path = "tibber/consumption_page_info.graphql",
    response_derives = "Debug"
)]
pub struct ConsumptionPageInfo;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "tibber/schema.json",
    query_path = "tibber/consumption_hourly_page_info.graphql",
    response_derives = "Debug"
)]
pub struct ConsumptionHourlyPageInfo;

/// `EnergyResolution` is an enum that represents the different energy resolution types.
#[derive(Debug, Clone, PartialEq)]
pub enum EnergyResolution {
    Hourly,
    Daily,
    Monthly,
    Yearly,
}

impl EnergyResolution {
    pub fn to_consumption_resolution(&self) -> consumption::EnergyResolution {
        match self {
            EnergyResolution::Hourly => consumption::EnergyResolution::HOURLY,
            EnergyResolution::Daily => consumption::EnergyResolution::DAILY,
            EnergyResolution::Monthly => consumption::EnergyResolution::MONTHLY,
            EnergyResolution::Yearly => consumption::EnergyResolution::ANNUAL,
        }
    }

    pub fn to_consumption_page_info_resolution(&self) -> consumption_page_info::EnergyResolution {
        match self {
            EnergyResolution::Hourly => consumption_page_info::EnergyResolution::HOURLY,
            EnergyResolution::Daily => consumption_page_info::EnergyResolution::DAILY,
            EnergyResolution::Monthly => consumption_page_info::EnergyResolution::MONTHLY,
            EnergyResolution::Yearly => consumption_page_info::EnergyResolution::ANNUAL,
        }
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

/// Trait to define the common fields required for parsing price info.
pub trait PriceInfoFields {
    fn total(&self) -> Option<f64>;
    fn energy(&self) -> Option<f64>;
    fn tax(&self) -> Option<f64>;
    fn starts_at(&self) -> Option<String>;
    fn level(&self) -> PriceLevel;
    fn currency(&self) -> String;
}

// Trait implementations for PriceInfoFields
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

impl PriceInfoFields for price_hourly::PriceHourlyViewerHomeCurrentSubscriptionPriceInfoTomorrow {
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

// Trait implementations for ConsumptionPageInfoFields
impl ConsumptionPageInfoFields
    for consumption_page_info::ConsumptionPageInfoViewerHomeConsumptionPageInfo
{
    fn start_cursor(&self) -> Option<String> {
        self.start_cursor.clone()
    }
    fn has_previous_page(&self) -> bool {
        self.has_previous_page.unwrap()
    }
    fn count(&self) -> usize {
        self.count.unwrap() as usize
    }
    fn total_cost(&self) -> Option<f64> {
        self.total_cost
    }
    fn total_consumption(&self) -> Option<f64> {
        self.total_consumption
    }
    fn currency(&self) -> Option<String> {
        self.currency.clone()
    }
}

impl ConsumptionPageInfoFields
    for consumption_hourly_page_info::ConsumptionHourlyPageInfoViewerHomeConsumptionPageInfo
{
    fn start_cursor(&self) -> Option<String> {
        self.start_cursor.clone()
    }
    fn has_previous_page(&self) -> bool {
        self.has_previous_page.unwrap()
    }
    fn count(&self) -> usize {
        self.count.unwrap() as usize
    }
    fn total_cost(&self) -> Option<f64> {
        self.total_cost
    }
    fn total_consumption(&self) -> Option<f64> {
        self.total_consumption
    }
    fn currency(&self) -> Option<String> {
        self.currency.clone()
    }
}

/// Trait to define the common fields for consumption page information.
pub trait ConsumptionPageInfoFields {
    fn start_cursor(&self) -> Option<String>;
    fn has_previous_page(&self) -> bool;
    fn count(&self) -> usize;
    fn total_cost(&self) -> Option<f64>;
    fn total_consumption(&self) -> Option<f64>;
    fn currency(&self) -> Option<String>;
}

/// Trait for providing consumption page data and related cost data
#[async_trait::async_trait]
pub trait ConsumptionPageProvider: Send + Sync {
    /// Fetches the consumption page information for the last `n` data points at the specified energy resolution.
    async fn get_consumption_page_info(
        &self,
        access_config: &AccessConfig,
        n: i64,
        cursor: Option<String>,
        energy_resolution: EnergyResolution,
    ) -> Result<ConsumptionPage, Box<dyn std::error::Error + Send + Sync>>;

    /// Fetches the cost data for the last 12 months.
    async fn get_cost_last_12_months(
        &self,
        access_config: &AccessConfig,
        estimated_daily_fee: &Option<f64>,
    ) -> Result<
        Option<(Vec<f64>, String, DateTime<FixedOffset>)>,
        Box<dyn std::error::Error + Send + Sync>,
    >;
}

/// `AccessConfig` is a struct that represents the configuration for accessing the Tibber service.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AccessConfig {
    pub token: String,
    pub url: String,
    pub home_id: String,
    pub reconnect_timeout: u64,
}

impl Default for AccessConfig {
    fn default() -> Self {
        AccessConfig {
            token: "3A77EECF61BD445F47241A5A36202185C35AF3AF58609E19B53F3A8872AD7BE1-1".to_string(),
            url: "https://api.tibber.com/v1-beta/gql".to_string(),
            home_id: "96a14971-525a-4420-aae9-e5aedaa129ff".to_string(),
            reconnect_timeout: 120,
        }
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
    pub fn parse_price_info(price_info: &impl PriceInfoFields) -> Option<Self> {
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
    pub fn new_current(
        price_info: price_current::PriceCurrentViewerHomeCurrentSubscriptionPriceInfoCurrent,
    ) -> Option<Self> {
        Self::parse_price_info(&price_info)
    }

    pub fn handle_missing_entries(mut price_infos: Vec<PriceInfo>) -> Option<Vec<PriceInfo>> {
        // Handle DST transitions
        match price_infos.len() {
            // DST start (23 hours) - add zero entry for missing hour
            23 => {
                let hours: HashSet<u32> = price_infos.iter().map(|p| p.starts_at.hour()).collect();

                // Find missing hour (0-24)
                let missing_hour = (0..24).find(|h| !hours.contains(h)).unwrap_or(0);
                let timezone = Local;

                // Create zero entry for missing hour
                let currency = price_infos
                    .first()
                    .map(|p| p.currency.clone())
                    .unwrap_or_default();
                price_infos.push(PriceInfo {
                    total: 0.0,
                    energy: 0.0,
                    tax: 0.0,
                    starts_at: Local::now()
                        .date_naive()
                        .and_hms_opt(missing_hour, 0, 0)?
                        .and_local_timezone(timezone.offset_from_utc_date(&Utc::now().date_naive()))
                        .single()?,
                    currency,
                    level: PriceLevel::None,
                });

                // Sort by hour
                price_infos.sort_by(|a, b| a.starts_at.hour().cmp(&b.starts_at.hour()));
                Some(price_infos)
            }
            // Normal day (24 hours) and DST end (25 hours) - keep all entries
            _ => Some(price_infos),
        }
    }

    pub fn new_hourly_today(
        price_info_today: Vec<
            Option<price_hourly::PriceHourlyViewerHomeCurrentSubscriptionPriceInfoToday>,
        >,
    ) -> Option<Vec<PriceInfo>> {
        let mut price_infos = Vec::new();

        for hour in price_info_today {
            if let Some(current_hour) = hour {
                if let Some(price_info) = Self::parse_price_info(&current_hour) {
                    price_infos.push(price_info);
                }
            }
        }

        Self::handle_missing_entries(price_infos)
    }

    pub fn new_hourly_tomorrow(
        price_info_tomorrow: Vec<
            Option<price_hourly::PriceHourlyViewerHomeCurrentSubscriptionPriceInfoTomorrow>,
        >,
    ) -> Option<Vec<PriceInfo>> {
        let mut price_infos = Vec::new();

        for hour in price_info_tomorrow {
            if let Some(current_hour) = hour {
                if let Some(price_info) = Self::parse_price_info(&current_hour) {
                    price_infos.push(price_info);
                }
            }
        }

        match price_infos.len() {
            0 => None,
            _ => Some(price_infos),
        }
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
        consumption_page: &impl ConsumptionPageInfoFields,
    ) -> Option<ConsumptionPage> {
        let start_cursor = consumption_page.start_cursor().unwrap_or_default();
        let has_previous_page = consumption_page.has_previous_page();
        let count = consumption_page.count();
        let total_cost = consumption_page.total_cost()?;
        let total_consumption = consumption_page.total_consumption()?;
        let currency = String::from(consumption_page.currency()?);

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
    pub fn parse_consumption_node(
        consumption_data: &consumption::ConsumptionViewerHomeConsumptionNodes,
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
    pub fn new_nodes(
        consumption_list: consumption::ConsumptionViewerHomeConsumption,
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
    pub fn new_nodes_today(
        consumption_list: consumption::ConsumptionViewerHomeConsumption,
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
