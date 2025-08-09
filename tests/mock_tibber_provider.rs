use chrono::{DateTime, FixedOffset, Local};
use tibberator::tibber::{AccessConfig, LiveMeasurementSubscription, TibberDataProvider};

pub struct MockTibberDataProvider;

#[async_trait::async_trait]
impl TibberDataProvider for MockTibberDataProvider {
    async fn get_consumption_data_today(
        &self,
        _config: &AccessConfig,
    ) -> Result<Option<(Vec<f64>, String, DateTime<FixedOffset>)>, Box<dyn std::error::Error>> {
        Ok(Some((vec![1.0, 2.0, 3.0], "Mock Consumption".to_string(), Local::now().fixed_offset())))
    }
    
    async fn get_cost_data_today(
        &self,
        _config: &AccessConfig,
        _estimated_daily_fee: &Option<f64>,
    ) -> Result<Option<(Vec<f64>, String, DateTime<FixedOffset>)>, Box<dyn std::error::Error>> {
        Ok(Some((vec![10.0, 20.0, 30.0], "Mock Cost".to_string(), Local::now().fixed_offset())))
    }
    
    async fn get_cost_last_30_days(
        &self,
        _config: &AccessConfig,
        _estimated_daily_fee: &Option<f64>,
    ) -> Result<Option<(Vec<f64>, String, DateTime<FixedOffset>)>, Box<dyn std::error::Error>> {
        Ok(Some((vec![100.0, 200.0, 300.0], "Mock Cost Last 30 Days".to_string(), Local::now().fixed_offset())))
    }
    
    async fn get_cost_last_12_months(
        &self,
        _config: &AccessConfig,
        _estimated_daily_fee: &Option<f64>,
    ) -> Result<Option<(Vec<f64>, String, DateTime<FixedOffset>)>, Box<dyn std::error::Error>> {
        Ok(Some((vec![1000.0, 2000.0, 3000.0], "Mock Cost Last 12 Months".to_string(), Local::now().fixed_offset())))
    }
    
    async fn get_cost_all_years(
        &self,
        _config: &AccessConfig,
        _estimated_daily_fee: &Option<f64>,
    ) -> Result<Option<(Vec<f64>, String, DateTime<FixedOffset>)>, Box<dyn std::error::Error>> {
        Ok(Some((vec![10000.0, 20000.0, 30000.0], "Mock Cost All Years".to_string(), Local::now().fixed_offset())))
    }
    
    async fn get_prices_today_tomorrow(
        &self,
        _config: &AccessConfig,
    ) -> Result<
        Option<((Vec<f64>, Vec<f64>), String, DateTime<FixedOffset>)>,
        Box<dyn std::error::Error>,
    > {
        Ok(Some(((vec![10.0, 15.0], vec![20.0, 25.0]), "Mock Prices".to_string(), Local::now().fixed_offset())))
    }
    
    async fn connect_live_measurement(
        &self,
        _config: &AccessConfig,
    ) -> Result<LiveMeasurementSubscription, Box<dyn std::error::Error + Send + Sync>> {
        // Return an error for the mock implementation, which is sufficient for testing
        // error handling paths in the application
        Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            "Mock subscription - simulating connection failure for testing"
        )))
    }
}
