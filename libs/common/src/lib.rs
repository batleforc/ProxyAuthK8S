use std::env;

use kube::Client;
use tracing::{info, instrument};

use deadpool_redis::{Config, Pool, Runtime};

#[derive(Clone)]
pub struct State {
    pub client: Client,
    redis: Pool,
}

impl State {
    #[instrument(name = "StateInit")]
    pub async fn new() -> Self {
        let cfg =
            Config::from_url(env::var("REDIS_URL").unwrap_or("redis://127.0.0.1:6379".to_string()));
        let pool = cfg.create_pool(Some(Runtime::Tokio1)).unwrap();
        info!("Connected to Redis");
        let client = Client::try_default()
            .await
            .expect("failed to create kube Client");
        info!("Connected to Kubernetes");
        Self {
            client,
            redis: pool,
        }
    }

    #[instrument(name = "GetRedisConn", skip(self))]
    pub async fn get_redis_conn(
        &self,
    ) -> Result<deadpool_redis::Connection, deadpool_redis::PoolError> {
        self.redis.get().await
    }
}
