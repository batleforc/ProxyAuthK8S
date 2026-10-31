//! The `TokenExchange` mode (RFC 0010): the proxy exchanges the caller's proxy
//! token for a token of the cluster's IdP (RFC 8693) and relays that one.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::authentication_configuration::AuthenticationConfiguration;

fn default_max_cache_seconds() -> u32 {
    300
}

/// A form value CR, LF or `&` could split: refused, never escaped (§4.3).
const FORM_VALUE_RULE: &str = "!self.matches('[\\\\r\\\\n&]')";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct TokenExchangeSettings {
    /// The `audience` parameter of the exchange; empty: omitted.
    #[serde(default)]
    #[schemars(length(max = 512), extend("x-kubernetes-validations" = [
        serde_json::json!({ "rule": FORM_VALUE_RULE, "message": "audience must not carry CR, LF or &" }),
    ]))]
    pub audience: String,
    /// The `scope` parameter of the exchange; empty: omitted.
    #[serde(default)]
    #[schemars(length(max = 512), extend("x-kubernetes-validations" = [
        serde_json::json!({ "rule": FORM_VALUE_RULE, "message": "scope must not carry CR, LF or &" }),
    ]))]
    pub scope: String,
    /// Dex only: the upstream connector that issued the proxy's token;
    /// empty: omitted.
    #[serde(default)]
    #[schemars(length(max = 256), extend("x-kubernetes-validations" = [
        serde_json::json!({ "rule": FORM_VALUE_RULE, "message": "connector_id must not carry CR, LF or &" }),
    ]))]
    pub connector_id: String,
    /// The longest an exchanged token is reused, 0 to 3600 seconds; `0`
    /// exchanges on every request. Bounds how long a group removed at the
    /// IdP can go unseen by the apiserver.
    #[serde(default = "default_max_cache_seconds")]
    #[schemars(range(max = 3600))]
    pub max_cache_seconds: u32,
}

impl Default for TokenExchangeSettings {
    fn default() -> Self {
        Self {
            audience: String::new(),
            scope: String::new(),
            connector_id: String::new(),
            max_cache_seconds: default_max_cache_seconds(),
        }
    }
}

impl TokenExchangeSettings {
    /// The mirror of the admission rules, so a CR that bypassed admission
    /// still fails in its status.
    ///
    /// # Errors
    ///
    /// The first rule of RFC 0010 §4.3 broken.
    pub fn validate(&self, auth: Option<&AuthenticationConfiguration>) -> Result<(), String> {
        let Some(auth) = auth else {
            return Err(
                "TokenExchange needs auth_config with an enabled oidc_provider: it is the client that exchanges"
                    .to_string(),
            );
        };
        if auth.disable_validation {
            return Err(
                "TokenExchange refuses disable_validation: any bearer would get a token of the cluster"
                    .to_string(),
            );
        }
        if !auth.oidc_provider.enabled {
            return Err(
                "TokenExchange needs oidc_provider.enabled: it is the client that exchanges"
                    .to_string(),
            );
        }
        if self.max_cache_seconds > 3600 {
            return Err("token_exchange.max_cache_seconds must be at most 3600".to_string());
        }
        for (field, value) in [
            ("audience", &self.audience),
            ("scope", &self.scope),
            ("connector_id", &self.connector_id),
        ] {
            if value.contains(['\r', '\n', '&']) {
                return Err(format!("token_exchange.{field} must not carry CR, LF or &"));
            }
        }
        Ok(())
    }

    /// The operator warnings of RFC 0010 §4.3, given the issuer of the
    /// proxy's own IdP and the cluster's resolved client secret.
    #[must_use]
    pub fn warnings(
        &self,
        proxy_issuer: &str,
        cluster_issuer: &str,
        has_client_secret: bool,
    ) -> Vec<String> {
        let mut warnings = Vec::new();
        let same_issuer =
            proxy_issuer.trim_end_matches('/') == cluster_issuer.trim_end_matches('/');
        if !same_issuer && self.connector_id.is_empty() {
            warnings.push(format!(
                "token_exchange: the cluster's IdP ({cluster_issuer}) is not the proxy's and connector_id is empty; the exchange relies on that IdP trusting the proxy's tokens"
            ));
        }
        if !has_client_secret {
            warnings.push(
                "token_exchange: oidc_provider has no client secret (inline or config_from); most IdPs refuse an exchange from a public client"
                    .to_string(),
            );
        }
        warnings
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auth(enabled: bool, disable_validation: bool) -> AuthenticationConfiguration {
        serde_json::from_value(serde_json::json!({
            "disable_validation": disable_validation,
            "validate_against": "Kubernetes",
            "oidc_provider": {
                "enabled": enabled,
                "issuer_url": "https://sso.example.com",
                "client_id": "kube",
            },
        }))
        .unwrap()
    }

    #[test]
    fn an_absent_block_takes_the_documented_defaults() {
        let settings: TokenExchangeSettings =
            serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(settings, TokenExchangeSettings::default());
        assert_eq!(settings.max_cache_seconds, 300);
    }

    #[test]
    fn an_exchange_without_a_client_or_a_validated_user_is_refused() {
        let settings = TokenExchangeSettings::default();
        assert!(settings.validate(None).is_err());
        assert!(settings.validate(Some(&auth(false, false))).is_err());
        assert!(settings.validate(Some(&auth(true, true))).is_err());
        assert!(settings.validate(Some(&auth(true, false))).is_ok());
    }

    #[test]
    fn a_form_value_that_could_split_the_form_is_refused() {
        for bad in ["a&b", "a\rb", "a\nb"] {
            let settings = TokenExchangeSettings {
                audience: bad.to_string(),
                ..TokenExchangeSettings::default()
            };
            assert!(
                settings.validate(Some(&auth(true, false))).is_err(),
                "{bad:?}"
            );
        }
        let settings = TokenExchangeSettings {
            max_cache_seconds: 3601,
            ..TokenExchangeSettings::default()
        };
        assert!(settings.validate(Some(&auth(true, false))).is_err());
    }

    /// T8: a foreign issuer without a connector warns.
    #[test]
    fn a_foreign_issuer_without_connector_warns() {
        let settings = TokenExchangeSettings::default();
        assert!(
            settings
                .warnings("https://sso/", "https://sso", true)
                .is_empty()
        );
        let warnings = settings.warnings("https://sso", "https://other", true);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("connector_id"));
        let federated = TokenExchangeSettings {
            connector_id: "proxy".to_string(),
            ..TokenExchangeSettings::default()
        };
        assert!(
            federated
                .warnings("https://sso", "https://other", true)
                .is_empty()
        );
        assert_eq!(
            settings.warnings("https://sso", "https://sso", false).len(),
            1
        );
    }
}
