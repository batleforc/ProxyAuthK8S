//! Resolving an [`super::OidcProvider`] block from an external Secret.
//!
//! Keeping the OIDC client credentials in the `ProxyKubeApi` spec means the
//! secret is readable by anyone with `get` on the CR, ends up in `kubectl get -o
//! yaml`, and is copied into every backup of the resource. `config_from` points
//! at a Secret instead, so the CR carries only a reference and the credential
//! lives in the one object cluster operators already protect (and that
//! external-secrets/ESO already knows how to populate).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

mod error;
pub use error::OidcConfigError;

/// The Secret keys read out of a `config_from` reference.
///
/// They are exactly the snake_case field names of [`super::OidcProvider`], so a
/// Secret is written the same way the inline block is.
pub const ISSUER_URL_KEY: &str = "issuer_url";
pub const CLIENT_ID_KEY: &str = "client_id";
pub const CLIENT_SECRET_KEY: &str = "client_secret";
pub const AUDIENCE_KEY: &str = "audience";
pub const EXTRA_SCOPE_KEY: &str = "extra_scope";

/// Where an [`super::OidcProvider`] block is read from.
///
/// Only `Secret` is offered: the block carries `client_secret`, and a ConfigMap
/// is not an appropriate store for a credential. Non-secret fields that an
/// operator wants in plain sight can simply stay inline in the CR — a value set
/// inline is used whenever the Secret does not carry that key.
///
/// An enum rather than a bare struct so a future non-Secret backend can be added
/// without breaking the serialized shape, mirroring
/// [`crate::certificate::CertSource`].
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub enum OidcConfigSource {
    /// Read the provider block from the keys of a Secret.
    Secret {
        name: String,
        /// Defaults to the `ProxyKubeApi`'s own namespace. Honoured only when
        /// `PROXYAUTH_ALLOW_CROSS_NS_OIDC` is set — see
        /// [`cross_namespace_oidc_allowed`].
        namespace: Option<String>,
    },
}

/// The subset of an [`super::OidcProvider`] a Secret may supply.
///
/// `None` means "the Secret did not carry this key", which is what makes the
/// inline CR value the fallback rather than an empty override.
#[derive(Clone, Default)]
pub struct OidcProviderOverrides {
    pub issuer_url: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub audience: Option<String>,
    pub extra_scope: Option<String>,
}

/// Hand-written for the same reason as [`super::OidcProvider`]'s: this struct
/// holds the resolved plaintext client secret, and must never widen a log line
/// into a credential leak.
impl std::fmt::Debug for OidcProviderOverrides {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OidcProviderOverrides")
            .field("issuer_url", &self.issuer_url)
            .field("client_id", &self.client_id)
            .field(
                "client_secret",
                &self.client_secret.as_ref().map(|_| "***REDACTED***"),
            )
            .field("audience", &self.audience)
            .field("extra_scope", &self.extra_scope)
            .finish()
    }
}

/// Whether a `config_from` reference may read a Secret from a namespace other
/// than the owning `ProxyKubeApi`'s.
///
/// Defaults to `false` (secure by default), for the same reason as the cert
/// equivalent: the read uses the controller's cluster-wide `ServiceAccount`, so
/// allowing an arbitrary `namespace` turns a tenant who can create
/// `ProxyKubeApi` objects into a cross-namespace Secret read oracle — and here
/// the resolved value is an OAuth client secret rather than a CA bundle.
///
/// Deliberately a *separate* knob from `PROXYAUTH_ALLOW_CROSS_NS_CERT`: a
/// cluster that shares one CA bundle across namespaces has no reason to also
/// share OIDC credentials, and vice versa, so opting into one must not silently
/// opt into the other.
#[must_use]
pub fn cross_namespace_oidc_allowed() -> bool {
    std::env::var("PROXYAUTH_ALLOW_CROSS_NS_OIDC").is_ok_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "true" | "1" | "yes" | "on" | "enabled"
        )
    })
}

/// Resolve the namespace a `config_from` Secret is actually read from, applying
/// the cross-namespace policy above.
///
/// Deliberately silent: it is also called by [`OidcConfigSource::cache_key`],
/// which runs on every proxied request even when the Secret itself is served
/// from cache — logging here would emit one warning per request. The denial is
/// logged once per actual read, in [`OidcConfigSource::resolve`].
fn resolve_oidc_namespace<'a>(requested: Option<&'a str>, cr_ns: &'a str) -> &'a str {
    match requested {
        Some(requested) if requested != cr_ns && !cross_namespace_oidc_allowed() => cr_ns,
        Some(requested) => requested,
        None => cr_ns,
    }
}

impl OidcConfigSource {
    /// A stable identity for what this reference actually reads, for caching.
    ///
    /// Includes the namespace *after* the cross-namespace policy has been
    /// applied, so a denied reference and an allowed one to the same name never
    /// share an entry — and so editing the reference on a live CR changes the
    /// key rather than serving the old Secret until a TTL expires.
    #[must_use]
    pub fn cache_key(&self, cr_ns: &str) -> String {
        match self {
            OidcConfigSource::Secret { name, namespace } => {
                let target_ns = resolve_oidc_namespace(namespace.as_deref(), cr_ns);
                format!("secret/{target_ns}/{name}")
            }
        }
    }

    /// Read the referenced Secret and return the fields it carries.
    ///
    /// Keys the Secret does not carry come back as `None` so the caller falls
    /// back to the inline CR value. Keys the Secret carries that are *not* one
    /// of the five above are ignored rather than rejected: a `config_from`
    /// Secret is routinely managed by external-secrets and shared with other
    /// consumers, so extra keys are normal and must not fail the whole block.
    pub async fn resolve(
        &self,
        client: kube::Client,
        cr_ns: &str,
    ) -> Result<OidcProviderOverrides, OidcConfigError> {
        match self {
            OidcConfigSource::Secret { name, namespace } => {
                let target_ns = resolve_oidc_namespace(namespace.as_deref(), cr_ns);
                if let Some(requested) = namespace.as_deref()
                    && requested != target_ns
                {
                    tracing::warn!(
                        requested,
                        cr_ns,
                        "cross-namespace OIDC config reference denied by \
                         PROXYAUTH_ALLOW_CROSS_NS_OIDC; pinning to the resource namespace"
                    );
                }
                let secrets: kube::Api<k8s_openapi::api::core::v1::Secret> =
                    kube::Api::namespaced(client, target_ns);
                let secret = secrets
                    .get(name)
                    .await
                    .map_err(|source| OidcConfigError::Read {
                        name: name.clone(),
                        namespace: target_ns.to_string(),
                        source: Box::new(source),
                    })?;

                let mut overrides = OidcProviderOverrides::default();
                let mut saw_any_key = false;

                // `Secret.data` is already base64-decoded by the k8s client on
                // deserialization (`ByteString`); decoding again would corrupt
                // any value outside the base64 alphabet. `stringData` is
                // write-only (the apiserver folds it into `data` before
                // persisting) but is honoured too, for the off chance a read
                // ever returns it populated.
                if let Some(data) = &secret.data {
                    for (key, value) in data {
                        let value = String::from_utf8(value.0.clone()).map_err(|source| {
                            OidcConfigError::Utf8 {
                                key: key.clone(),
                                name: name.clone(),
                                source,
                            }
                        })?;
                        saw_any_key |= overrides.apply(key, value);
                    }
                }
                if let Some(string_data) = &secret.string_data {
                    for (key, value) in string_data {
                        saw_any_key |= overrides.apply(key, value.clone());
                    }
                }

                if !saw_any_key {
                    // Every key was unrecognised (or the Secret was empty): the
                    // reference resolves to nothing at all, which is almost
                    // always a mistyped key name. Say so instead of silently
                    // falling back to the inline block the operator was trying
                    // to replace.
                    return Err(OidcConfigError::NoRecognisedKey {
                        name: name.clone(),
                        namespace: target_ns.to_string(),
                    });
                }
                Ok(overrides)
            }
        }
    }
}

impl OidcProviderOverrides {
    /// Record `key` if it is one this block understands. Returns whether it was.
    ///
    /// `pub(crate)` so the key-name mapping has exactly one definition that both
    /// the resolution path and the merge tests go through — a test that hardcodes
    /// the field assignments would keep passing if a key were ever renamed here.
    pub(crate) fn apply(&mut self, key: &str, value: String) -> bool {
        match key {
            ISSUER_URL_KEY => self.issuer_url = Some(value),
            CLIENT_ID_KEY => self.client_id = Some(value),
            CLIENT_SECRET_KEY => self.client_secret = Some(value),
            AUDIENCE_KEY => self.audience = Some(value),
            EXTRA_SCOPE_KEY => self.extra_scope = Some(value),
            _ => return false,
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, MutexGuard};

    use super::{OidcConfigSource, OidcProviderOverrides, resolve_oidc_namespace};

    /// Serializes the tests that mutate the process environment. Under
    /// cargo-nextest each test is its own process, but a plain `cargo test` runs
    /// them as threads of one process, where concurrent `set_var`/`var` on the
    /// same variable would race.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Takes the environment lock, ignoring poisoning: a panicking test has
    /// already failed, and the guard below still restored what it changed.
    fn lock_env() -> MutexGuard<'static, ()> {
        ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Restores `PROXYAUTH_ALLOW_CROSS_NS_OIDC` to whatever it held before.
    struct EnvGuard {
        previous: Option<String>,
    }

    impl EnvGuard {
        const KEY: &'static str = "PROXYAUTH_ALLOW_CROSS_NS_OIDC";

        fn set(value: Option<&str>) -> Self {
            let guard = EnvGuard {
                previous: std::env::var(Self::KEY).ok(),
            };
            // SAFETY: the caller holds the environment lock (see `lock_env`).
            unsafe {
                match value {
                    Some(value) => std::env::set_var(Self::KEY, value),
                    None => std::env::remove_var(Self::KEY),
                }
            }
            guard
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // SAFETY: the caller holds the environment lock (see `lock_env`).
            unsafe {
                match &self.previous {
                    Some(value) => std::env::set_var(Self::KEY, value),
                    None => std::env::remove_var(Self::KEY),
                }
            }
        }
    }

    #[test]
    fn every_documented_key_maps_to_its_field() {
        let mut overrides = OidcProviderOverrides::default();
        for key in [
            super::ISSUER_URL_KEY,
            super::CLIENT_ID_KEY,
            super::CLIENT_SECRET_KEY,
            super::AUDIENCE_KEY,
            super::EXTRA_SCOPE_KEY,
        ] {
            assert!(overrides.apply(key, format!("value-of-{key}")), "{key}");
        }

        assert_eq!(overrides.issuer_url.as_deref(), Some("value-of-issuer_url"));
        assert_eq!(overrides.client_id.as_deref(), Some("value-of-client_id"));
        assert_eq!(
            overrides.client_secret.as_deref(),
            Some("value-of-client_secret")
        );
        assert_eq!(overrides.audience.as_deref(), Some("value-of-audience"));
        assert_eq!(
            overrides.extra_scope.as_deref(),
            Some("value-of-extra_scope")
        );
    }

    /// A `config_from` Secret managed by external-secrets is routinely shared
    /// with other consumers, so an unrelated key must not fail the block — and
    /// must not be mistaken for one that was understood.
    #[test]
    fn an_unrecognised_key_is_ignored_and_reported_as_such() {
        let mut overrides = OidcProviderOverrides::default();
        assert!(!overrides.apply("tls.crt", "whatever".to_string()));
        assert!(!overrides.apply("clientSecret", "camelCase is not a key".to_string()));
        assert!(overrides.client_secret.is_none());
    }

    #[test]
    fn a_reference_without_a_namespace_reads_from_the_resource_namespace() {
        let _lock = lock_env();
        let _guard = EnvGuard::set(None);

        assert_eq!(resolve_oidc_namespace(None, "tenant-a"), "tenant-a");
    }

    #[test]
    fn the_same_namespace_is_never_treated_as_a_cross_namespace_read() {
        let _lock = lock_env();
        let _guard = EnvGuard::set(None);

        assert_eq!(
            resolve_oidc_namespace(Some("tenant-a"), "tenant-a"),
            "tenant-a"
        );
    }

    /// Secure by default: without the opt-in, a reference naming someone else's
    /// namespace is pinned back to the CR's own rather than honoured, so a
    /// tenant who can create a `ProxyKubeApi` cannot read another namespace's
    /// OAuth client secret.
    #[test]
    fn a_cross_namespace_reference_is_denied_by_default() {
        let _lock = lock_env();
        let _guard = EnvGuard::set(None);

        assert_eq!(
            resolve_oidc_namespace(Some("tenant-b"), "tenant-a"),
            "tenant-a"
        );
    }

    #[test]
    fn a_cross_namespace_reference_is_honoured_once_opted_in() {
        let _lock = lock_env();
        let _guard = EnvGuard::set(Some("true"));

        assert_eq!(
            resolve_oidc_namespace(Some("tenant-b"), "tenant-a"),
            "tenant-b"
        );
    }

    /// The cert knob must not silently opt OIDC credentials in — they are
    /// separate decisions, which is the whole reason for a second variable.
    #[test]
    fn the_cert_knob_does_not_open_the_oidc_one() {
        let _lock = lock_env();
        let _guard = EnvGuard::set(None);
        let cert_previous = std::env::var("PROXYAUTH_ALLOW_CROSS_NS_CERT").ok();
        // SAFETY: the environment lock is held above.
        unsafe { std::env::set_var("PROXYAUTH_ALLOW_CROSS_NS_CERT", "true") };

        let resolved = resolve_oidc_namespace(Some("tenant-b"), "tenant-a");

        // SAFETY: the environment lock is held above.
        unsafe {
            match cert_previous {
                Some(value) => std::env::set_var("PROXYAUTH_ALLOW_CROSS_NS_CERT", value),
                None => std::env::remove_var("PROXYAUTH_ALLOW_CROSS_NS_CERT"),
            }
        }
        assert_eq!(resolved, "tenant-a");
    }

    #[test]
    fn only_the_documented_truthy_spellings_open_the_gate() {
        for value in ["true", "1", "yes", "on", "enabled", "TRUE", " True "] {
            let _lock = lock_env();
            let _guard = EnvGuard::set(Some(value));
            assert_eq!(
                resolve_oidc_namespace(Some("tenant-b"), "tenant-a"),
                "tenant-b",
                "{value} should open the gate"
            );
        }
        for value in ["false", "0", "no", "off", "", "maybe"] {
            let _lock = lock_env();
            let _guard = EnvGuard::set(Some(value));
            assert_eq!(
                resolve_oidc_namespace(Some("tenant-b"), "tenant-a"),
                "tenant-a",
                "{value} should not open the gate"
            );
        }
    }

    /// The serialized shape is CRD API surface: it is what operators write in
    /// YAML and what already-applied resources deserialize from.
    #[test]
    fn the_reference_deserializes_from_the_documented_yaml_shape() {
        let source: OidcConfigSource = serde_json::from_value(serde_json::json!({
            "Secret": { "name": "oidc-config" }
        }))
        .expect("a name-only reference should deserialize");
        let OidcConfigSource::Secret { name, namespace } = &source;
        assert_eq!(name, "oidc-config");
        assert_eq!(namespace.as_deref(), None);

        let source: OidcConfigSource = serde_json::from_value(serde_json::json!({
            "Secret": { "name": "oidc-config", "namespace": "shared" }
        }))
        .expect("a reference with a namespace should deserialize");
        let OidcConfigSource::Secret { name, namespace } = &source;
        assert_eq!(name, "oidc-config");
        assert_eq!(namespace.as_deref(), Some("shared"));
    }

    /// The reference is only names, so it stays readable — but the resolved
    /// values are credentials and must not be.
    #[test]
    fn debug_shows_the_reference_but_redacts_the_resolved_values() {
        let source = OidcConfigSource::Secret {
            name: "oidc-config".to_string(),
            namespace: Some("shared".to_string()),
        };
        let rendered = format!("{source:?}");
        assert!(rendered.contains("oidc-config"), "{rendered}");
        assert!(rendered.contains("shared"), "{rendered}");

        let mut overrides = OidcProviderOverrides::default();
        overrides.apply("client_secret", "unmistakable-client-secret".to_string());
        let rendered = format!("{overrides:?}");
        assert!(
            !rendered.contains("unmistakable-client-secret"),
            "{rendered}"
        );
        assert!(rendered.contains("***REDACTED***"), "{rendered}");
    }
}
