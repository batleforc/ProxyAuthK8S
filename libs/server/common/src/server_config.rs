//! HTTP(S) listener settings and the rustls server configuration built from them.

use rustls::pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject as _};

use crate::{config, error::TlsConfigError};

#[derive(Clone)]
pub struct ServerConfig {
    pub port: u16,
    pub https: bool,
    pub cert_path: Option<String>,
    pub key_path: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl ServerConfig {
    /// Listener settings from the process-wide [`config::Config`]
    /// (`SERVER_PORT`, `SERVER_HTTPS`, `SERVER_CERT_PATH`, `SERVER_KEY_PATH`).
    #[must_use]
    pub fn new() -> Self {
        let server = &config::get().server;
        Self {
            port: server.port,
            https: server.https,
            cert_path: server.cert_path.clone(),
            key_path: server.key_path.clone(),
        }
    }

    /// Build the rustls server configuration from the configured certificate and
    /// private-key files.
    ///
    /// # Errors
    ///
    /// Returns [`TlsConfigError`] when HTTPS is enabled but a path is missing, a
    /// PEM file cannot be read, or rustls rejects the certificate/key pair.
    pub fn rustls_config(&self) -> Result<rustls::ServerConfig, TlsConfigError> {
        let cert_path = self
            .cert_path
            .as_ref()
            .ok_or(TlsConfigError::MissingPath("SERVER_CERT_PATH"))?;
        let key_path = self
            .key_path
            .as_ref()
            .ok_or(TlsConfigError::MissingPath("SERVER_KEY_PATH"))?;

        let cert_chain = CertificateDer::pem_file_iter(cert_path)
            .map_err(|err| TlsConfigError::Cert(err.to_string()))?
            .flatten()
            .collect();
        let key_der = PrivateKeyDer::from_pem_file(key_path)
            .map_err(|err| TlsConfigError::Key(err.to_string()))?;
        Ok(rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(cert_chain, key_der)?)
    }
}
