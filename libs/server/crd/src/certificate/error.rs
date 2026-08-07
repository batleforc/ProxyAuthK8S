//! Error type for resolving a [`super::CertSource`] against the cluster.

/// Failure while resolving a certificate source (Secret, ConfigMap, or inline).
///
/// Carries enough context (which kind, which object, which key) to explain the
/// failure in a controller status or a server log, without collapsing every
/// cause into an opaque string.
#[derive(Debug, thiserror::Error)]
pub enum CertError {
    /// The backing Secret or ConfigMap could not be read from the API server.
    #[error("failed to read {kind} {name}: {source}")]
    Read {
        kind: &'static str,
        name: String,
        #[source]
        source: kube::Error,
    },
    /// The named key is absent from the Secret/ConfigMap data.
    #[error("key {key} not found in {kind} {name}")]
    KeyNotFound {
        kind: &'static str,
        key: String,
        name: String,
    },
    /// The Secret/ConfigMap carries no data at all.
    #[error("no data found in {kind} {name}")]
    NoData { kind: &'static str, name: String },
    /// The certificate bytes were not valid base64.
    #[error("failed to base64-decode certificate: {0}")]
    Base64(#[from] base64::DecodeError),
    /// The decoded certificate was not valid UTF-8.
    #[error("certificate is not valid UTF-8: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),
    /// A required half of an mTLS client certificate resolved to nothing.
    #[error("mTLS client {half} resolved to nothing")]
    Empty { half: &'static str },
}

#[cfg(test)]
mod tests {
    use super::CertError;

    #[test]
    fn cert_error_messages_name_the_kind_and_object() {
        assert_eq!(
            CertError::KeyNotFound {
                kind: "secret",
                key: "tls.crt".to_string(),
                name: "ca".to_string(),
            }
            .to_string(),
            "key tls.crt not found in secret ca"
        );
        assert_eq!(
            CertError::NoData {
                kind: "configmap",
                name: "ca".to_string(),
            }
            .to_string(),
            "no data found in configmap ca"
        );
        assert_eq!(
            CertError::Empty { half: "key" }.to_string(),
            "mTLS client key resolved to nothing"
        );
    }
}
