use crate::default::{default_audience_match_policy, default_egress_selector, default_empty_array};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The token issuer a [`super::JWTAuthenticator`] trusts, mirroring the
/// apiserver's `issuer`.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[schemars(extend("x-kubernetes-validations" = [
    serde_json::json!({
        "rule": "self.url.startsWith('https://')",
        "message": "issuer.url must be an https:// URL",
    }),
    serde_json::json!({
        "rule": "size(self.audiences) > 0",
        "message": "issuer.audiences must list at least one audience",
    }),
    serde_json::json!({
        "rule": "!has(self.discovery_url) || self.discovery_url.startsWith('https://')",
        "message": "issuer.discovery_url must be an https:// URL",
    }),
]))]
pub struct Issuer {
    /// The `iss` claim every token must carry, and the base for OIDC discovery
    /// unless `discovery_url` overrides it.
    #[schemars(length(max = 253))]
    pub url: String,
    /// Where the discovery document lives, when it is not `url` +
    /// `/.well-known/openid-configuration`.
    #[serde(alias = "discoveryURL")]
    #[schemars(length(max = 253))]
    pub discovery_url: Option<String>,
    /// PEM bundle used to verify the issuer's TLS certificate. Absent means the
    /// system trust store.
    #[serde(alias = "certificateAuthority")]
    pub certificate_authority: Option<String>,
    /// Accepted `aud` values. At least one — an authenticator that accepts any
    /// audience is the audience-confusion hole this whole area exists to close.
    #[serde(default = "default_empty_array::<String>")]
    pub audiences: Vec<String>,
    #[serde(
        alias = "audienceMatchPolicy",
        default = "default_audience_match_policy"
    )]
    pub audience_match_policy: AudienceMatchPolicyType,
    #[serde(alias = "egressSelector", default = "default_egress_selector")]
    pub egress_selector: EgressSelectorType,
}

#[derive(Serialize, Deserialize, Clone, Debug, Copy, PartialEq, Eq, JsonSchema)]
pub enum AudienceMatchPolicyType {
    #[serde(rename = "MatchAny")]
    MatchAny,
}

#[derive(Serialize, Deserialize, Clone, Debug, Copy, PartialEq, Eq, JsonSchema)]
pub enum EgressSelectorType {
    #[serde(rename = "controlplane")]
    ControlPlane,
    #[serde(rename = "cluster")]
    Cluster,
}

impl Issuer {
    /// The URL the discovery document is fetched from.
    #[must_use]
    pub fn discovery_endpoint(&self) -> String {
        match &self.discovery_url {
            Some(url) => url.clone(),
            None => format!(
                "{}/.well-known/openid-configuration",
                self.url.trim_end_matches('/')
            ),
        }
    }

    /// Whether `audiences` accepts this token's `aud` values.
    ///
    /// `MatchAny` — the only policy the apiserver defines — means the
    /// intersection must be non-empty.
    #[must_use]
    pub fn accepts_audience(&self, token_audiences: &[String]) -> bool {
        self.audiences
            .iter()
            .any(|accepted| token_audiences.iter().any(|value| value == accepted))
    }

    pub fn validate(&self) -> Result<(), String> {
        if !self.url.starts_with("https://") {
            return Err(format!(
                "issuer.url must be an https:// URL, got {}",
                self.url
            ));
        }
        // `discovery_url` is fetched by the proxy from its own network position,
        // and the JWKS it names decides which keys verify tokens. Left
        // unchecked, a `url` that passes admission plus a plaintext or
        // link-local `discovery_url` would point key discovery wherever the CR
        // author liked.
        if let Some(discovery_url) = &self.discovery_url
            && !discovery_url.starts_with("https://")
        {
            return Err(format!(
                "issuer.discovery_url must be an https:// URL, got {discovery_url}"
            ));
        }
        if self.audiences.is_empty() {
            return Err("issuer.audiences must list at least one audience".to_string());
        }
        Ok(())
    }
}
