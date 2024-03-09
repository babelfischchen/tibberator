use async_graphql::{EmptyMutation, Object, Schema, SimpleObject, Subscription, ID};
use async_graphql_axum::{GraphQLRequest, GraphQLResponse, GraphQLSubscription};
use axum::{routing::post, Extension, Router};
use chrono::offset::Local;
use futures::{Stream, StreamExt};
use tokio::sync::broadcast::Sender;
use tokio_stream::wrappers::BroadcastStream;

pub type TibberSchema = Schema<QueryRoot, EmptyMutation, SubscriptionRoot>;

pub struct SubscriptionServer {
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    port: u16,
    sender: Sender<LiveMeasurement>,
}

impl Drop for SubscriptionServer {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            shutdown.send(()).ok();
        }
    }
}

impl SubscriptionServer {
    pub async fn start() -> SubscriptionServer {
        let (channel, _) = tokio::sync::broadcast::channel(16);

        let schema = Schema::build(
            QueryRoot,
            EmptyMutation,
            SubscriptionRoot {
                channel: channel.clone(),
            },
        )
        .finish();

        let app = Router::new()
            .route("/", post(graphql_handler))
            .route_service("/ws", GraphQLSubscription::new(schema.clone()))
            .layer(Extension(schema));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let (shutdown_sender, shutdown_receiver) = tokio::sync::oneshot::channel::<()>();

        tokio::spawn(async move {
            axum::serve(listener, app.with_state(()))
                .with_graceful_shutdown(async move {
                    shutdown_receiver.await.ok();
                })
                .await
                .unwrap();
        });

        SubscriptionServer {
            port,
            shutdown: Some(shutdown_sender),
            sender: channel,
        }
    }

    pub fn websocket_url(&self) -> String {
        format!("ws://localhost:{}/ws", self.port)
    }

    pub fn send(
        &self,
        live_measurement: LiveMeasurement,
    ) -> Result<(), tokio::sync::broadcast::error::SendError<LiveMeasurement>> {
        self.sender.send(live_measurement).map(|_| ())
    }
}

#[axum_macros::debug_handler]
async fn graphql_handler(schema: Extension<TibberSchema>, req: GraphQLRequest) -> GraphQLResponse {
    schema.execute(req.into_inner()).await.into()
}

pub struct SubscriptionRoot {
    channel: Sender<LiveMeasurement>,
}

pub struct QueryRoot;

#[Object]
impl QueryRoot {
    pub async fn id(&self) -> ID {
        "123".into()
    }
}

#[derive(Clone, Debug, SimpleObject)]
pub struct LiveMeasurement {
    pub timestamp: String,
    pub power: f64,
    pub last_meter_consumption: f64,
    pub accumulated_consumption: f64,
    pub accumulated_production: f64,
    pub accumulated_consumption_last_hour: f64,
    pub accumulated_production_last_hour: f64,
    pub accumulated_cost: f64,
    pub accumulated_reward: f64,
    pub currency: String,
    pub min_power: f64,
    pub average_power: f64,
    pub max_power: f64,
    pub power_production: f64,
    pub power_reactive: f64,
    pub power_production_reactive: f64,
    pub min_power_production: f64,
    pub max_power_production: f64,
    pub last_meter_production: f64,
    pub power_factor: f64,
    pub voltage_phase1: f64,
    pub voltage_phase2: f64,
    pub voltage_phase3: f64,
    pub current_phase1: f64,
    pub current_l1: f64,
    pub current_phase2: f64,
    pub current_l2: f64,
    pub current_phase3: f64,
    pub current_l3: f64,
    pub signal_strength: i64,
}

impl Default for LiveMeasurement {
    fn default() -> Self {
        LiveMeasurement {
            timestamp: Local::now().format("%+").to_string(),
            power: 0.0,
            last_meter_consumption: 0.0,
            accumulated_consumption: 0.0,
            accumulated_production: 0.0,
            accumulated_consumption_last_hour: 0.0,
            accumulated_production_last_hour: 0.04,
            accumulated_cost: 0.0,
            accumulated_reward: 0.0,
            currency: "EUR".to_string(),
            min_power: 0.0,
            average_power: 0.0,
            max_power: 0.0,
            power_production: 0.0,
            power_reactive: 0.0,
            power_production_reactive: 0.0,
            min_power_production: 0.0,
            max_power_production: 0.0,
            last_meter_production: 0.0,
            power_factor: 0.0,
            voltage_phase1: 0.0,
            voltage_phase2: 0.0,
            voltage_phase3: 0.0,
            current_phase1: 0.0,
            current_l1: 0.0,
            current_phase2: 0.0,
            current_l2: 0.0,
            current_phase3: 0.0,
            current_l3: 0.0,
            signal_strength: -63,
        }
    }
}

#[Subscription]
impl SubscriptionRoot {
    async fn live_measurement(&self, _home_id: ID) -> impl Stream<Item = LiveMeasurement> {
        println!("Subscription received");
        BroadcastStream::new(self.channel.subscribe())
            .filter_map(|result| async move { result.ok() })
    }
}
