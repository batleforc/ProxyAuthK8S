use super::oidc_config_source::{OidcConfigError, OidcConfigSource, OidcProviderOverrides};
use crate::default::{default_disabled, default_empty_string};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// An enabled provider that cannot be contacted is a cluster that never
/// authenticates anyone, so the apiserver refuses the CR outright.
#[derive(Serialize, Deserialize, Clone, JsonSchema)]
#[schemars(extend("x-kubernetes-validations" = [
    serde_json::json!({
        "rule": "!self.enabled || has(self.config_from) || (self.issuer_url != '' && self.client_id != '')",
        "message": "an enabled OIDC provider requires a non-empty issuer_url and client_id, or a config_from reference supplying them",
    }),
    serde_json::json!({
        "rule": "!self.expose_oauth_authorization_server || self.enabled",
        "message": "expose_oauth_authorization_server requires the OIDC provider to be enabled",
    }),
]))]
pub struct OidcProvider {
    #[serde(default = "default_disabled")]
    pub enabled: bool,
    /// Ignored when the `config_from` Secret carries an `issuer_url` key.
    /// Optional only so a `config_from`-only block need not restate it; an
    /// enabled provider must end up with a non-empty value from one side or the
    /// other.
    #[serde(default = "default_empty_string")]
    pub issuer_url: String,
    /// Ignored when the `config_from` Secret carries a `client_id` key. See
    /// `issuer_url` for why it is optional.
    #[serde(default = "default_empty_string")]
    pub client_id: String,
    /// The OAuth client secret, inline.
    ///
    /// Prefer `config_from`: a value here is readable by anyone with `get` on
    /// this resource, shows up in `kubectl get -o yaml`, and is copied into
    /// every backup of it.
    pub client_secret: Option<String>,
    #[serde(default = "default_empty_string")]
    pub extra_scope: String,
    /// Audience the access token must carry (its `aud` claim) to be accepted.
    ///
    /// When empty, `client_id` is used — correct for providers that put the
    /// client id in `aud`. Set it explicitly when the provider mints tokens for a
    /// distinct resource audience, so `client_id` (who the token is for) and the
    /// expected audience (what the token is for) are configured independently.
    #[serde(default = "default_empty_string")]
    pub audience: String,
    /// Accept a token whose `aud` does not name this service but whose
    /// `azp`/`client_id` does.
    ///
    /// Off by default: `azp`/`client_id` identify the client the token was issued
    /// to, not the resource, so accepting them weakens audience validation.
    /// Enable only for providers (e.g. Keycloak) that mint self-audience tokens
    /// carrying the client only in `azp`.
    #[serde(default = "default_disabled")]
    pub accept_authorized_party: bool,
    /// Expose a mediated OAuth 2.0 Authorization Server (RFC 6749 + PKCE) for
    /// this cluster, advertised at `/.well-known/oauth-authorization-server`
    /// (RFC 8414), like OpenShift does for its own OAuth server.
    ///
    /// The proxy itself becomes the authorization server an external caller
    /// talks to — `issuer`, `authorization_endpoint`, `token_endpoint`, and
    /// `jwks_uri` are all this cluster's own proxy URLs — mediating the whole
    /// exchange with the configured `issuer_url` server-side. A caller (e.g.
    /// `oc login`, or any RFC-8414-aware tool) never needs to be registered
    /// with — or even learn the hostname of — the upstream provider, and no
    /// companion `oauth2-proxy` is needed to serve this document.
    ///
    /// Off by default and unauthenticated by nature (discovery, and the
    /// authorization/token endpoints it advertises, must be reachable before
    /// a caller has a token): enabling it lets anonymous callers learn that
    /// this cluster exists and start a login against it. `redirect_uri` at
    /// `/oauth/authorize` is restricted to loopback addresses
    /// (`http://localhost`/`http://127.0.0.1`, RFC 8252) since callers are
    /// not pre-registered — accepting an arbitrary `redirect_uri` would be an
    /// open redirect. Requires `enabled: true`.
    #[serde(default = "default_disabled")]
    pub expose_oauth_authorization_server: bool,
    /// Read the provider block from an external Secret instead of inlining it
    /// here.
    ///
    /// The Secret's keys are the snake_case field names of this struct —
    /// `issuer_url`, `client_id`, `client_secret`, `audience`, `extra_scope` —
    /// and any key it carries wins over the inline value; keys it does not carry
    /// fall back to the inline value, so a partial Secret (the common case: just
    /// `client_secret`) works. Keys outside that set are ignored, since a Secret
    /// managed by external-secrets is routinely shared with other consumers.
    ///
    /// The policy toggles (`enabled`, `accept_authorized_party`,
    /// `expose_oauth_authorization_server`) are deliberately *not* readable from
    /// the Secret: they decide how strictly tokens are validated and who may
    /// reach this cluster, so they must stay visible in the CR that admission
    /// and review look at.
    pub config_from: Option<OidcConfigSource>,
}

impl OidcProvider {
    /// Resolve this block against the cluster, applying a `config_from` Secret
    /// over the inline values.
    ///
    /// Returns the provider as it should actually be used. Without a
    /// `config_from` this is a clone and touches the apiserver not at all, so
    /// the common inline case pays nothing.
    ///
    /// Fails closed: a Secret that cannot be read, or a block still missing
    /// `issuer_url`/`client_id` after the merge, is an error rather than a
    /// half-configured OIDC client that would reject every token at runtime for
    /// a reason nobody can see.
    pub async fn resolve(
        &self,
        client: kube::Client,
        cr_ns: &str,
    ) -> Result<Self, OidcConfigError> {
        let Some(source) = &self.config_from else {
            return Ok(self.clone());
        };

        let overrides = source.resolve(client, cr_ns).await?;
        self.merge(overrides)
    }

    /// Apply an already-read [`OidcProviderOverrides`] over the inline block.
    ///
    /// Split out of [`Self::resolve`] so the precedence and completeness rules —
    /// the part that decides which credential is actually used — are testable
    /// without an apiserver.
    pub fn merge(&self, overrides: OidcProviderOverrides) -> Result<Self, OidcConfigError> {
        let mut resolved = self.clone();
        if let Some(issuer_url) = overrides.issuer_url {
            resolved.issuer_url = issuer_url;
        }
        if let Some(client_id) = overrides.client_id {
            resolved.client_id = client_id;
        }
        if let Some(client_secret) = overrides.client_secret {
            resolved.client_secret = Some(client_secret);
        }
        if let Some(audience) = overrides.audience {
            resolved.audience = audience;
        }
        if let Some(extra_scope) = overrides.extra_scope {
            resolved.extra_scope = extra_scope;
        }

        // What the CEL rule can no longer check once `config_from` is set.
        if resolved.enabled {
            if resolved.issuer_url.is_empty() {
                return Err(OidcConfigError::Incomplete {
                    field: "issuer_url",
                });
            }
            if resolved.client_id.is_empty() {
                return Err(OidcConfigError::Incomplete { field: "client_id" });
            }
        }
        Ok(resolved)
    }
}

/// Hand-written so the inline `client_secret` — a plaintext OAuth client secret
/// carried in the CR spec — is never written to logs when a `ProxyKubeApi` is
/// `Debug`-formatted. The proxy hot path records the whole resource as a span
/// field (`fields(proxy = ?ctx.proxy)` in the redirect workers, and
/// `debug!(proxy = ?proxy)` when the cluster is resolved), so a derived `Debug`
/// here leaks the secret to the trace backend on every proxied request.
///
/// Whether a secret is *set* stays visible (`Some("***REDACTED***")` vs `None`)
/// — that distinction is what makes a misconfigured provider debuggable, and it
/// reveals nothing. Mirrors [`crate::certificate::CertSource`]'s impl and
/// `common::oidc_conf::OidcConf`'s, which redact for the same reason.
impl std::fmt::Debug for OidcProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Destructured so a new field cannot be added without deciding here
        // whether it is safe to print.
        let Self {
            enabled,
            issuer_url,
            client_id,
            client_secret,
            extra_scope,
            audience,
            accept_authorized_party,
            expose_oauth_authorization_server,
            config_from,
        } = self;
        f.debug_struct("OidcProvider")
            .field("enabled", enabled)
            .field("issuer_url", issuer_url)
            .field("client_id", client_id)
            .field(
                "client_secret",
                &client_secret.as_ref().map(|_| "***REDACTED***"),
            )
            .field("extra_scope", extra_scope)
            .field("audience", audience)
            .field("accept_authorized_party", accept_authorized_party)
            .field(
                "expose_oauth_authorization_server",
                expose_oauth_authorization_server,
            )
            // Only a name/namespace reference, so it stays visible.
            .field("config_from", config_from)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::super::oidc_config_source::{OidcConfigError, OidcProviderOverrides};
    use super::OidcProvider;

    fn provider_with_secret(secret: Option<&str>) -> OidcProvider {
        OidcProvider {
            enabled: true,
            issuer_url: "https://issuer.example.com".to_string(),
            client_id: "proxy-auth-k8s".to_string(),
            client_secret: secret.map(ToString::to_string),
            extra_scope: "groups".to_string(),
            audience: "proxy-auth-k8s".to_string(),
            accept_authorized_party: false,
            expose_oauth_authorization_server: false,
            config_from: None,
        }
    }

    #[test]
    fn debug_redacts_the_client_secret() {
        let rendered = format!("{:?}", provider_with_secret(Some("s3cr3t-value")));
        assert!(
            !rendered.contains("s3cr3t-value"),
            "client_secret leaked into Debug output: {rendered}"
        );
        assert!(rendered.contains("***REDACTED***"), "{rendered}");
    }

    #[test]
    fn debug_still_distinguishes_a_set_secret_from_an_unset_one() {
        let with = format!("{:?}", provider_with_secret(Some("s3cr3t-value")));
        let without = format!("{:?}", provider_with_secret(None));
        assert!(with.contains("client_secret: Some("), "{with}");
        assert!(without.contains("client_secret: None"), "{without}");
    }

    fn overrides(pairs: &[(&str, &str)]) -> OidcProviderOverrides {
        let mut overrides = OidcProviderOverrides::default();
        for (key, value) in pairs {
            assert!(
                overrides.apply(key, (*value).to_string()),
                "{key} is not a recognised key"
            );
        }
        overrides
    }

    #[test]
    fn a_provider_without_config_from_merges_to_itself() {
        let provider = provider_with_secret(Some("inline-secret"));
        let merged = provider.merge(OidcProviderOverrides::default()).unwrap();

        assert_eq!(merged.issuer_url, "https://issuer.example.com");
        assert_eq!(merged.client_id, "proxy-auth-k8s");
        assert_eq!(merged.client_secret.as_deref(), Some("inline-secret"));
    }

    #[test]
    fn a_secret_key_wins_over_the_inline_value() {
        let merged = provider_with_secret(Some("inline-secret"))
            .merge(overrides(&[("client_secret", "secret-from-the-secret")]))
            .unwrap();

        assert_eq!(
            merged.client_secret.as_deref(),
            Some("secret-from-the-secret")
        );
    }

    /// The common case: only the credential is externalised, everything else
    /// stays readable in the CR.
    #[test]
    fn keys_absent_from_the_secret_fall_back_to_the_inline_values() {
        let merged = provider_with_secret(None)
            .merge(overrides(&[("client_secret", "secret-from-the-secret")]))
            .unwrap();

        assert_eq!(merged.issuer_url, "https://issuer.example.com");
        assert_eq!(merged.client_id, "proxy-auth-k8s");
        assert_eq!(merged.audience, "proxy-auth-k8s");
        assert_eq!(merged.extra_scope, "groups");
    }

    #[test]
    fn every_supported_key_is_applied() {
        let merged = provider_with_secret(None)
            .merge(overrides(&[
                ("issuer_url", "https://from-secret.example.com"),
                ("client_id", "id-from-secret"),
                ("client_secret", "secret-from-secret"),
                ("audience", "audience-from-secret"),
                ("extra_scope", "scope-from-secret"),
            ]))
            .unwrap();

        assert_eq!(merged.issuer_url, "https://from-secret.example.com");
        assert_eq!(merged.client_id, "id-from-secret");
        assert_eq!(merged.client_secret.as_deref(), Some("secret-from-secret"));
        assert_eq!(merged.audience, "audience-from-secret");
        assert_eq!(merged.extra_scope, "scope-from-secret");
    }

    /// The policy toggles decide how strictly tokens are validated, so a Secret
    /// must not be able to move them — `merge` only ever touches the five
    /// data fields.
    #[test]
    fn merging_never_moves_the_policy_toggles() {
        let mut provider = provider_with_secret(None);
        provider.enabled = true;
        provider.accept_authorized_party = true;
        provider.expose_oauth_authorization_server = true;

        let merged = provider
            .merge(overrides(&[("client_secret", "secret-from-secret")]))
            .unwrap();

        assert!(merged.enabled);
        assert!(merged.accept_authorized_party);
        assert!(merged.expose_oauth_authorization_server);
    }

    /// `config_from` relaxes the CEL rule that would otherwise require these
    /// inline, so the runtime has to be the one that refuses an incomplete
    /// block — rather than building an OIDC client that rejects every token.
    #[test]
    fn an_enabled_provider_still_incomplete_after_the_merge_is_refused() {
        let mut provider = provider_with_secret(None);
        provider.enabled = true;
        provider.issuer_url = String::new();

        let err = provider
            .merge(overrides(&[("client_secret", "secret-from-secret")]))
            .unwrap_err();
        assert!(
            matches!(
                err,
                OidcConfigError::Incomplete {
                    field: "issuer_url"
                }
            ),
            "unexpected error: {err}"
        );

        let mut provider = provider_with_secret(None);
        provider.enabled = true;
        provider.client_id = String::new();
        let err = provider
            .merge(overrides(&[(
                "issuer_url",
                "https://from-secret.example.com",
            )]))
            .unwrap_err();
        assert!(
            matches!(err, OidcConfigError::Incomplete { field: "client_id" }),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn the_secret_can_supply_the_fields_the_cr_leaves_empty() {
        let mut provider = provider_with_secret(None);
        provider.enabled = true;
        provider.issuer_url = String::new();
        provider.client_id = String::new();

        let merged = provider
            .merge(overrides(&[
                ("issuer_url", "https://from-secret.example.com"),
                ("client_id", "id-from-secret"),
            ]))
            .unwrap();

        assert_eq!(merged.issuer_url, "https://from-secret.example.com");
        assert_eq!(merged.client_id, "id-from-secret");
    }

    /// A disabled provider is never used, so an empty block is not an error —
    /// only an *enabled* one has to be complete.
    #[test]
    fn a_disabled_provider_is_not_required_to_be_complete() {
        let mut provider = provider_with_secret(None);
        provider.enabled = false;
        provider.issuer_url = String::new();
        provider.client_id = String::new();

        assert!(
            provider
                .merge(overrides(&[("audience", "whatever")]))
                .is_ok()
        );
    }

    #[test]
    fn debug_keeps_the_non_secret_fields_visible() {
        // These are what make a misconfigured provider diagnosable from a trace;
        // redacting the secret must not blank out the rest of the struct.
        let rendered = format!("{:?}", provider_with_secret(Some("s3cr3t-value")));
        for expected in [
            "enabled: true",
            "issuer_url: \"https://issuer.example.com\"",
            "client_id: \"proxy-auth-k8s\"",
            "extra_scope: \"groups\"",
            "audience: \"proxy-auth-k8s\"",
            "accept_authorized_party: false",
            "expose_oauth_authorization_server: false",
        ] {
            assert!(
                rendered.contains(expected),
                "missing {expected} in {rendered}"
            );
        }
    }
}
