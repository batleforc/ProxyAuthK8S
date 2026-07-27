//! Redis connectivity, single-node or cluster.
//!
//! The two deadpool flavours have distinct pool and connection types, so rather
//! than leaking that choice to every caller the pool is wrapped here and every
//! command goes through [`RedisPool::query`]. That leaves exactly one place
//! where the two modes differ.

use deadpool_redis::redis::{aio::ConnectionLike, Cmd, FromRedisValue, RedisError};
use deadpool_redis::{cluster, Config, Pool, Runtime};
use tracing::info;

/// Everything that can go wrong reaching Redis.
#[derive(Debug, thiserror::Error)]
pub enum RedisPoolError {
    #[error("could not build the redis pool: {0}")]
    Build(String),
    #[error("could not get a redis connection: {0}")]
    Pool(String),
    #[error("redis command failed: {0}")]
    Command(#[from] RedisError),
    #[error("a cached value could not be deserialized: {0}")]
    Deserialize(String),
}

#[derive(Clone)]
pub enum RedisPool {
    Single(Pool),
    Cluster(cluster::Pool),
}

impl RedisPool {
    /// Build the pool described by `url`.
    ///
    /// A comma-separated list of URLs, or `REDIS_CLUSTER=true`, selects cluster
    /// mode. Cluster mode with a single seed node is valid: the client
    /// discovers the rest of the topology itself.
    ///
    /// # Errors
    ///
    /// Returns [`RedisPoolError::Build`] when the pool cannot be created from the
    /// given URL(s).
    pub fn from_url(url: &str) -> Result<Self, RedisPoolError> {
        let urls: Vec<String> = url
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(str::to_string)
            .collect();

        let forced_cluster = std::env::var("REDIS_CLUSTER")
            .is_ok_and(|value| value.eq_ignore_ascii_case("true") || value == "1");

        if urls.len() > 1 || forced_cluster {
            info!(nodes = urls.len(), "Connecting to Redis in cluster mode");
            let config = cluster::Config::from_urls(urls);
            return config
                .create_pool(Some(Runtime::Tokio1))
                .map(RedisPool::Cluster)
                .map_err(|err| RedisPoolError::Build(err.to_string()));
        }

        info!("Connecting to Redis in single-node mode");
        Config::from_url(url)
            .create_pool(Some(Runtime::Tokio1))
            .map(RedisPool::Single)
            .map_err(|err| RedisPoolError::Build(err.to_string()))
    }

    #[must_use]
    pub fn is_cluster(&self) -> bool {
        matches!(self, RedisPool::Cluster(_))
    }

    /// Run one command, whichever mode the pool is in.
    ///
    /// # Errors
    ///
    /// Returns [`RedisPoolError`] when no connection can be obtained or the
    /// command fails.
    pub async fn query<T: FromRedisValue>(&self, command: &Cmd) -> Result<T, RedisPoolError> {
        match self {
            RedisPool::Single(pool) => {
                let mut conn = pool
                    .get()
                    .await
                    .map_err(|err| RedisPoolError::Pool(err.to_string()))?;
                run(command, &mut conn).await
            }
            RedisPool::Cluster(pool) => {
                let mut conn = pool
                    .get()
                    .await
                    .map_err(|err| RedisPoolError::Pool(err.to_string()))?;
                run(command, &mut conn).await
            }
        }
    }
}

async fn run<T: FromRedisValue>(
    command: &Cmd,
    conn: &mut impl ConnectionLike,
) -> Result<T, RedisPoolError> {
    command
        .query_async(conn)
        .await
        .map_err(RedisPoolError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_single_url_stays_single_node() {
        let pool = RedisPool::from_url("redis://127.0.0.1:6379").expect("pool should build");
        assert!(!pool.is_cluster());
    }

    #[test]
    fn a_comma_separated_list_selects_cluster_mode() {
        let pool = RedisPool::from_url("redis://node-a:6379,redis://node-b:6379")
            .expect("pool should build");
        assert!(pool.is_cluster());
    }

    #[test]
    fn surrounding_whitespace_is_ignored() {
        let pool = RedisPool::from_url(" redis://node-a:6379 , redis://node-b:6379 ")
            .expect("pool should build");
        assert!(pool.is_cluster());
    }
}
