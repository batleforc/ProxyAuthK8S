use std::env;

use kube::Client;
use r2d2::Pool;
use redis::Client as RedisClient;
use tracing::{info, instrument};

#[derive(Clone)]
pub struct State {
    pub client: Client,
    pub redis_pool: Pool<RedisClient>,
}

impl State {
    #[instrument(name = "StateInit")]
    pub async fn new() -> Self {
        let redis_client = RedisClient::open(
            env::var("REDIS_URL").unwrap_or("redis://127.0.0.1:6379".to_string()),
        )
        .expect("failed to create redis Client");
        let redis_pool = match Pool::builder().build(redis_client) {
            Ok(p) => p,
            Err(e) => panic!("failed to create redis Pool: {}", e),
        };
        info!("Connected to Redis");
        let client = Client::try_default()
            .await
            .expect("failed to create kube Client");
        info!("Connected to Kubernetes");
        Self { client, redis_pool }
    }
}
