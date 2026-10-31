//! How the proxy presents the caller's identity to the target apiserver
//! (RFC 0014).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::authentication_configuration::{ClaimMappings, ValidateAgainst};

pub mod front_proxy;
pub mod issuer_key;
pub mod token_exchange;

pub use token_exchange::TokenExchangeSettings;

/// `Passthrough` relays the caller's token, as before RFC 0014. The two others
/// make the proxy the apiserver's trusted authenticator: `FrontProxy` asserts
/// the identity in `X-Remote-*` headers over mutual TLS, `Issuer` in a JWT the
/// proxy signs. `TokenExchange` (RFC 0010) relays a token of the cluster's
/// IdP the proxy obtained for the caller: the apiserver trusts that IdP, not
/// the proxy.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, Hash, JsonSchema)]
pub enum UpstreamAuthMode {
    #[default]
    Passthrough,
    FrontProxy,
    Issuer,
    TokenExchange,
}

impl UpstreamAuthMode {
    /// Whether the apiserver trusts the proxy for the identity, rather than
    /// the caller's own token.
    #[must_use]
    pub fn is_trusted(self) -> bool {
        matches!(self, Self::FrontProxy | Self::Issuer)
    }

    /// Whether the caller's own token never reaches the apiserver. A release
    /// that does not know the mode must not serve the cluster as
    /// `Passthrough` (RFC 0014, decision 42).
    #[must_use]
    pub fn replaces_token(self) -> bool {
        self != Self::Passthrough
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Passthrough => "Passthrough",
            Self::FrontProxy => "FrontProxy",
            Self::Issuer => "Issuer",
            Self::TokenExchange => "TokenExchange",
        }
    }

    /// Parses the name as written in the CRD and in `UPSTREAM_AUTH_ALLOWED_MODES`.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "Passthrough" => Some(Self::Passthrough),
            "FrontProxy" => Some(Self::FrontProxy),
            "Issuer" => Some(Self::Issuer),
            "TokenExchange" => Some(Self::TokenExchange),
            _ => None,
        }
    }
}

/// The modes this version serves; grows with each phase of RFC 0014 (phase 3:
/// `Issuer`) and with RFC 0010 (`TokenExchange`). A mode outside it is
/// refused with `503`, never relayed as `Passthrough` (decision 34).
pub const SERVED_MODES: &[UpstreamAuthMode] = &[
    UpstreamAuthMode::Passthrough,
    UpstreamAuthMode::FrontProxy,
    UpstreamAuthMode::Issuer,
    UpstreamAuthMode::TokenExchange,
];

/// A prefix that is empty, starts with `system:`, carries a control character
/// or an edge space would forge reserved names or end up in a header.
const PREFIX_RULE: &str = "self.matches('[\\\\x00-\\\\x1f\\\\x7f]|^\\\\s|\\\\s$') == false \
     && !self.lowerAscii().startsWith('system:')";

#[derive(Serialize, Deserialize, Clone, Debug, Default, JsonSchema)]
#[schemars(extend("x-kubernetes-validations" = [
    serde_json::json!({
        "rule": "!has(self.mode) || self.mode == 'Passthrough' || self.mode == 'TokenExchange' || (has(self.user_prefix) && self.user_prefix != '' && has(self.group_prefix) && self.group_prefix != '')",
        "message": "user_prefix and group_prefix are required and non-empty in FrontProxy and Issuer",
    }),
    serde_json::json!({
        "rule": "!has(self.issuer_key) || !has(self.mode) || (self.mode != 'FrontProxy' && self.mode != 'TokenExchange')",
        "message": "issuer_key is for Issuer (or its preparation in Passthrough), not FrontProxy nor TokenExchange",
    }),
    serde_json::json!({
        "rule": "!has(self.mode) || self.mode != 'Issuer' || has(self.issuer_key)",
        "message": "mode Issuer requires issuer_key",
    }),
    // RFC 0010 §4.3: one source of truth for the `Authorization`.
    serde_json::json!({
        "rule": "(has(self.mode) && self.mode == 'TokenExchange') == has(self.token_exchange)",
        "message": "token_exchange is required in mode TokenExchange and refused in the other modes",
    }),
]))]
pub struct UpstreamAuth {
    /// Default: `Passthrough`, the behaviour without this block.
    #[serde(default)]
    pub mode: UpstreamAuthMode,
    /// Prepended to the username the apiserver sees. No default: a prefix is
    /// a collision decision nobody else can take.
    #[schemars(length(max = 64), extend("x-kubernetes-validations" = [
        serde_json::json!({ "rule": PREFIX_RULE, "message": "user_prefix must not start with system: nor carry a control character or an edge space" }),
    ]))]
    pub user_prefix: Option<String>,
    /// Prepended to every group the apiserver sees.
    #[schemars(length(max = 64), extend("x-kubernetes-validations" = [
        serde_json::json!({ "rule": PREFIX_RULE, "message": "group_prefix must not start with system: nor carry a control character or an edge space" }),
    ]))]
    pub group_prefix: Option<String>,
    /// The signing key of `Issuer`, also accepted in `Passthrough` so the
    /// apiserver can be prepared before the switch.
    pub issuer_key: Option<IssuerKeySettings>,
    /// The exchange of `TokenExchange` (RFC 0010); required in that mode and
    /// refused in the others.
    pub token_exchange: Option<TokenExchangeSettings>,
}

fn default_token_ttl_seconds() -> u32 {
    300
}

fn default_rotation_days() -> u32 {
    30
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct IssuerKeySettings {
    /// The issuer the apiserver declares: `<public proxy URL>/issuer/<ns>/<name>`.
    /// Written here, never derived from the environment, so a redeploy or a
    /// restore cannot change it.
    #[schemars(length(max = 2048), extend("x-kubernetes-validations" = [
        serde_json::json!({ "rule": "self.startsWith('https://')", "message": "issuer_url must be https" }),
        serde_json::json!({
            "rule": "!self.matches('^https://(localhost|127\\\\.|\\\\[::1\\\\])')",
            "message": "issuer_url must be reachable by the apiserver, not a loopback address",
        }),
    ]))]
    pub issuer_url: String,
    /// Lifetime of an issued JWT, 60 to 900 seconds.
    #[serde(default = "default_token_ttl_seconds")]
    #[schemars(range(min = 60, max = 900))]
    pub token_ttl_seconds: u32,
    /// Automatic key rotation, 1 to 365 days.
    #[serde(default = "default_rotation_days")]
    #[schemars(range(min = 1, max = 365))]
    pub rotation_days: u32,
}

/// The `kubernetes.io` and `k8s.io` domains, and their subdomains, are the
/// apiserver's own `extra` keys.
fn reserved_domain(domain: &str) -> bool {
    ["kubernetes.io", "k8s.io"]
        .iter()
        .any(|reserved| domain == *reserved || domain.ends_with(&format!(".{reserved}")))
}

/// Why an `extra` key cannot be forwarded to the apiserver, if it cannot:
/// upper case (the apiserver lowercases it), no domain, or a reserved domain.
#[must_use]
pub fn extra_key_refusal(key: &str) -> Option<&'static str> {
    if key.chars().any(|c| c.is_ascii_uppercase()) {
        return Some("is not lower case");
    }
    let Some((domain, _)) = key.split_once('/') else {
        return Some("has no domain prefix");
    };
    if reserved_domain(domain) {
        return Some("is in a domain reserved to Kubernetes");
    }
    None
}

fn check_prefix(field: &str, prefix: Option<&str>) -> Result<(), String> {
    let prefix = prefix.unwrap_or_default();
    if prefix.is_empty() {
        return Err(format!(
            "upstream_auth.{field} is required and non-empty in FrontProxy and Issuer"
        ));
    }
    let edge_space =
        prefix.starts_with(char::is_whitespace) || prefix.ends_with(char::is_whitespace);
    if prefix.chars().any(char::is_control)
        || edge_space
        || prefix.to_ascii_lowercase().starts_with("system:")
    {
        return Err(format!(
            "upstream_auth.{field} must not start with system: nor carry a control character or an edge space"
        ));
    }
    Ok(())
}

impl UpstreamAuth {
    /// The rules CEL cannot express (the extra keys, unbounded) next to the
    /// mirror of the ones it does, so a CR that bypassed admission still fails
    /// in its status.
    pub fn validate(
        &self,
        has_client_cert: bool,
        auth: Option<&crate::authentication_configuration::AuthenticationConfiguration>,
    ) -> Result<(), String> {
        if matches!(
            self.mode,
            UpstreamAuthMode::FrontProxy | UpstreamAuthMode::TokenExchange
        ) && self.issuer_key.is_some()
        {
            return Err(
                "issuer_key is for Issuer (or its preparation in Passthrough), not FrontProxy nor TokenExchange"
                    .to_string(),
            );
        }
        if (self.mode == UpstreamAuthMode::TokenExchange) != self.token_exchange.is_some() {
            return Err(
                "token_exchange is required in mode TokenExchange and refused in the other modes"
                    .to_string(),
            );
        }
        if let Some(exchange) = &self.token_exchange {
            exchange.validate(auth)?;
        }
        if self.mode == UpstreamAuthMode::Issuer && self.issuer_key.is_none() {
            return Err("mode Issuer requires issuer_key".to_string());
        }
        if !self.mode.is_trusted() {
            return Ok(());
        }
        check_prefix("user_prefix", self.user_prefix.as_deref())?;
        check_prefix("group_prefix", self.group_prefix.as_deref())?;
        if has_client_cert {
            return Err(
                "client_cert is refused in FrontProxy and Issuer: FrontProxy reads its certificate from the proxy's namespace, and in Issuer it would win over the JWT"
                    .to_string(),
            );
        }
        let Some(auth) = auth.filter(|auth| !auth.disable_validation) else {
            return Err(
                "FrontProxy and Issuer need a validated identity: set auth_config without disable_validation"
                    .to_string(),
            );
        };
        let mappings: Option<&ClaimMappings> = match auth.validate_against {
            ValidateAgainst::Kubernetes => {
                return Err(
                    "FrontProxy and Issuer cannot validate against Kubernetes: the SelfSubjectReview needs the apiserver to know the IdP"
                        .to_string(),
                );
            }
            ValidateAgainst::OidcProvider => {
                let Some(mappings) = auth.oidc_provider.claim_mappings.as_ref() else {
                    return Err(
                        "FrontProxy and Issuer require oidc_provider.claim_mappings: preferred_username must not become a trusted identity"
                            .to_string(),
                    );
                };
                Some(mappings)
            }
            ValidateAgainst::JwtAuthenticators => None,
        };
        let jwt_mappings = auth.jwt.iter().map(|jwt| &jwt.claim_mappings);
        for mappings in mappings.into_iter().chain(jwt_mappings) {
            for extra in &mappings.extra {
                if let Some(why) = extra_key_refusal(&extra.key) {
                    return Err(format!(
                        "claim_mappings.extra key {} {why}: it cannot be forwarded in FrontProxy or Issuer",
                        extra.key
                    ));
                }
            }
        }
        Ok(())
    }
}

/// The trust probe's verdict (RFC 0014 §5.4), as `status.upstreamAuth.trust`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, JsonSchema)]
pub enum UpstreamTrust {
    /// The apiserver saw the asserted identity, group and `extra` included.
    Trusted,
    /// It saw another identity.
    NotHonoured,
    /// It refused the proxy's certificate (`401`).
    Rejected,
    /// The probe reached no conclusion.
    Unknown,
    /// `Issuer`: the apiserver also took an unprefixed identity, its
    /// `userValidationRules` are missing (`IssuerNotTrusted`).
    Unguarded,
}

/// `status.upstreamAuth` (RFC 0014 §4.5). A `None` is serialised as `null`:
/// the status is published by merge patch, which would otherwise keep a
/// stale value.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpstreamAuthStatus {
    pub mode: UpstreamAuthMode,
    /// `FrontProxy`: the Secret to create, `<proxy namespace>/<name>`.
    #[serde(default)]
    pub client_cert_secret: Option<String>,
    /// `FrontProxy`: the value its target annotation must hold.
    #[serde(default)]
    pub client_cert_target: Option<String>,
    /// `FrontProxy`: the certificate's `notAfter`, RFC 3339.
    #[serde(default)]
    pub client_cert_not_after: Option<String>,
    /// `FrontProxy`: the certificate's subject.
    #[serde(default)]
    pub client_cert_subject: Option<String>,
    /// `Issuer`: `issuer_key.issuer_url`, mirrored.
    #[serde(default)]
    pub issuer_url: Option<String>,
    /// `Issuer`: the `aud` of every JWT, to declare in the apiserver.
    #[serde(default)]
    pub audience: Option<String>,
    /// `Issuer`: the JWKS the discovery document points to.
    #[serde(default)]
    pub jwks_uri: Option<String>,
    /// `Issuer`: the `kid` of the key that signs.
    #[serde(default)]
    pub key_id: Option<String>,
    /// `Issuer`: when that key began to sign, RFC 3339.
    #[serde(default)]
    pub key_rotated_at: Option<String>,
    /// The trust probe's last verdict; absent until a probe concludes, or
    /// when its last result is older than 15 minutes.
    #[serde(default)]
    pub trust: Option<UpstreamTrust>,
    /// When `trust` took its current value, RFC 3339.
    #[serde(default)]
    pub trust_since: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The defaults the CRD documents: a 5-minute JWT, a monthly rotation.
    #[test]
    fn an_issuer_key_without_ttl_or_rotation_takes_the_documented_defaults() {
        let settings: IssuerKeySettings = serde_json::from_value(
            serde_json::json!({ "issuer_url": "https://p.example/issuer/a/b" }),
        )
        .unwrap();
        assert_eq!(settings.token_ttl_seconds, 300);
        assert_eq!(settings.rotation_days, 30);
    }

    #[test]
    fn extra_keys_are_refused_when_the_apiserver_would_reinterpret_or_reserve_them() {
        assert_eq!(extra_key_refusal("example.com/team"), None);
        assert!(extra_key_refusal("Example.com/team").is_some());
        assert!(extra_key_refusal("email").is_some());
        assert!(extra_key_refusal("kubernetes.io/x").is_some());
        assert!(extra_key_refusal("authentication.kubernetes.io/credential-id").is_some());
        assert!(extra_key_refusal("foo.k8s.io/x").is_some());
        // A lookalike is not a subdomain.
        assert_eq!(extra_key_refusal("notkubernetes.io/x"), None);
    }

    #[test]
    fn a_prefix_must_be_safe_to_forward() {
        assert!(check_prefix("user_prefix", Some("proxyauth:")).is_ok());
        for bad in ["", "system:", "SYSTEM:x", " proxy:", "proxy: ", "pro\nxy:"] {
            assert!(check_prefix("user_prefix", Some(bad)).is_err(), "{bad:?}");
        }
        assert!(check_prefix("user_prefix", None).is_err());
    }

    #[test]
    fn modes_parse_from_their_crd_names_only() {
        for mode in [
            UpstreamAuthMode::Passthrough,
            UpstreamAuthMode::FrontProxy,
            UpstreamAuthMode::Issuer,
            UpstreamAuthMode::TokenExchange,
        ] {
            assert_eq!(UpstreamAuthMode::parse(mode.as_str()), Some(mode));
        }
        assert_eq!(UpstreamAuthMode::parse("frontproxy"), None);
        assert_eq!(UpstreamAuthMode::parse("tokenexchange"), None);
    }
}
