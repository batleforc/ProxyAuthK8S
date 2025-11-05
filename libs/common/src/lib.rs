use std::env;

use kube::Client;
use rustls::pki_types::{pem::PemObject as _, CertificateDer, PrivateKeyDer};
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

#[derive(Clone)]
pub struct ServerConfig {
    pub port: u16,
    pub https: bool,
    pub cert_path: Option<String>,
    pub key_path: Option<String>,
}

impl ServerConfig {
    pub fn new() -> Self {
        let port = env::var("SERVER_PORT")
            .unwrap_or("5437".to_string())
            .parse()
            .unwrap_or(5437);
        let https = env::var("SERVER_HTTPS")
            .unwrap_or("false".to_string())
            .parse()
            .unwrap_or(false);
        let cert_path = env::var("SERVER_CERT_PATH").ok();
        let key_path = env::var("SERVER_KEY_PATH").ok();
        Self {
            port,
            https,
            cert_path,
            key_path,
        }
    }

    pub fn get_rustls_config(&self) -> Option<rustls::ServerConfig> {
        if self.https {
            rustls::crypto::ring::default_provider()
                .install_default()
                .unwrap();

            // load TLS key/cert files
            let cert_chain = CertificateDer::pem_file_iter("cert.pem")
                .unwrap()
                .flatten()
                .collect();

            let key_der = PrivateKeyDer::from_pem_file("key.pem")
                .expect("Could not locate PKCS 8 private keys.");
            Some(
                rustls::ServerConfig::builder()
                    .with_no_client_auth()
                    .with_single_cert(cert_chain, key_der)
                    .unwrap(),
            )
        } else {
            None
        }
    }
}
