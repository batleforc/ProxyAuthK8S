//! Error type for resolving an [`super::OidcConfigSource`] against the cluster.

/// Failure while reading an OIDC provider block from an external Secret.
///
/// Carries which object and which key failed, so a controller status or a log
/// line can name the cause without collapsing it into an opaque string —
/// mirroring [`crate::certificate::CertError`].
///
/// No variant ever carries a *value* read from the Secret, only key names: these
/// errors are logged, and the values here are credentials.
#[derive(Debug, thiserror::Error)]
pub enum OidcConfigError {
    /// The referenced Secret could not be read from the API server.
    #[error("failed to read secret {namespace}/{name}: {source}")]
    Read {
        name: String,
        namespace: String,
        /// Boxed: `kube::Error` is large enough to bloat every `Result` in this
        /// module (clippy::result_large_err).
        #[source]
        source: Box<kube::Error>,
    },
    /// A value in the Secret was not valid UTF-8.
    #[error("value of key {key} in secret {name} is not valid UTF-8: {source}")]
    Utf8 {
        key: String,
        name: String,
        #[source]
        source: std::string::FromUtf8Error,
    },
    /// The Secret carried none of the keys this block understands.
    #[error(
        "secret {namespace}/{name} carries none of the expected keys \
         (issuer_url, client_id, client_secret, audience, extra_scope)"
    )]
    NoRecognisedKey { name: String, namespace: String },
    /// The block is still incomplete after merging the Secret over the inline
    /// values.
    ///
    /// Admission cannot catch this: `config_from` relaxes the CEL rule that
    /// requires a non-empty `issuer_url`/`client_id` inline, precisely because
    /// they are expected to arrive from the Secret. Only a live read can tell,
    /// so the controller resolves the reference during reconcile and writes this
    /// error into the resource status — a mistyped Secret name is visible in
    /// `kubectl get proxykubeapi` rather than only as every request failing to
    /// authenticate.
    #[error("OIDC provider is enabled but {field} is empty after resolving config_from")]
    Incomplete { field: &'static str },
}

#[cfg(test)]
mod tests {
    use super::OidcConfigError;

    #[test]
    fn error_messages_name_the_object_and_key() {
        assert_eq!(
            OidcConfigError::Utf8 {
                key: "client_secret".to_string(),
                name: "oidc-config".to_string(),
                source: String::from_utf8(vec![0xff]).unwrap_err(),
            }
            .to_string(),
            "value of key client_secret in secret oidc-config is not valid UTF-8: \
             invalid utf-8 sequence of 1 bytes from index 0"
        );
        assert_eq!(
            OidcConfigError::NoRecognisedKey {
                name: "oidc-config".to_string(),
                namespace: "default".to_string(),
            }
            .to_string(),
            "secret default/oidc-config carries none of the expected keys \
             (issuer_url, client_id, client_secret, audience, extra_scope)"
        );
        assert_eq!(
            OidcConfigError::Incomplete {
                field: "issuer_url"
            }
            .to_string(),
            "OIDC provider is enabled but issuer_url is empty after resolving config_from"
        );
    }

    /// The values read out of a `config_from` Secret are credentials and these
    /// errors are logged, so no variant may carry one.
    #[test]
    fn errors_never_carry_a_secret_value() {
        let rendered = OidcConfigError::Utf8 {
            key: "client_secret".to_string(),
            name: "oidc-config".to_string(),
            source: String::from_utf8(b"s3cr3t-\xff".to_vec()).unwrap_err(),
        }
        .to_string();
        assert!(!rendered.contains("s3cr3t"), "{rendered}");
    }
}
