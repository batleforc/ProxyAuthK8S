//! Rate limiting and fail2login, backed by Redis counters.
//!
//! Redis rather than in-process state because the backend is horizontally
//! scaled: a per-pod counter would let a caller multiply its budget by the
//! number of replicas, and a ban would only apply to whichever pod saw the
//! failures.
//!
//! The policy itself (which limit applies, how long a ban lasts) lives on
//! `SecurityConfiguration` and is unit-tested there; this module is only the
//! Redis plumbing around it.

use common::State;
use crd::{ProxyKubeApi, security::SecurityConfiguration};
use tracing::{debug, warn};

/// Window over which requests are counted.
const RATE_LIMIT_WINDOW_SECONDS: i64 = 60;

/// How long failed authentications are remembered.
///
/// Long enough that a slow brute force still trips the threshold, short enough
/// that an honest user who mistyped once is not held against forever.
const FAILURE_WINDOW_SECONDS: i64 = 3600;

fn rate_limit_key(proxy: &ProxyKubeApi, subject: &str) -> String {
    format!("proxyk8sauth:ratelimit:{}:{}", proxy.to_path(), subject)
}

fn failure_key(proxy: &ProxyKubeApi, subject: &str) -> String {
    format!("proxyk8sauth:fail2login:{}:{}", proxy.to_path(), subject)
}

fn ban_key(proxy: &ProxyKubeApi, subject: &str) -> String {
    format!("proxyk8sauth:ban:{}:{}", proxy.to_path(), subject)
}

/// Why a request was refused, and for how long the caller should wait.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Throttled {
    Banned { retry_after: Option<u64> },
    RateLimited { limit: u32, retry_after: u64 },
}

impl Throttled {
    #[must_use]
    pub fn retry_after(&self) -> Option<u64> {
        match self {
            Throttled::Banned { retry_after } => *retry_after,
            Throttled::RateLimited { retry_after, .. } => Some(*retry_after),
        }
    }

    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Throttled::Banned { .. } => {
                "too many failed authentications, this client is temporarily banned".to_string()
            }
            Throttled::RateLimited { limit, .. } => {
                format!("rate limit of {limit} requests per minute exceeded")
            }
        }
    }
}

fn security_config(proxy: &ProxyKubeApi) -> Option<&SecurityConfiguration> {
    proxy.spec.security_config.as_ref()
}

/// Whether a Redis failure should deny the request (fail closed) instead of
/// letting it through. Off by default: a Redis blip would otherwise become a
/// full outage. Set `THROTTLE_FAIL_CLOSED=true` on deployments that would rather
/// reject traffic than lose brute-force / rate-limit protection during a Redis
/// degradation.
///
/// Read from the central configuration (`common::config`).
fn fail_closed() -> bool {
    common::config::get().proxy.throttle_fail_closed
}

/// Whether `subject` is currently banned on this cluster.
///
/// A Redis failure fails open: refusing every request because the counter store
/// is unavailable would turn a Redis blip into a full outage, and the proxy
/// already returns 503 when it genuinely cannot serve.
pub async fn is_banned(state: &State, proxy: &ProxyKubeApi, subject: &str) -> bool {
    let Some(config) = security_config(proxy) else {
        return false;
    };
    if !config.fail2login_enabled() {
        return false;
    }

    match state.key_exists(&ban_key(proxy, subject)).await {
        Ok(banned) => banned,
        Err(err) => {
            if fail_closed() {
                warn!(%err, "ban state unavailable; failing closed (refusing the request)");
                true
            } else {
                warn!(%err, "could not read the ban state, letting the request through");
                false
            }
        }
    }
}

/// Remaining ban duration, for the `Retry-After` header.
pub async fn ban_retry_after(state: &State, proxy: &ProxyKubeApi, subject: &str) -> Option<u64> {
    state.key_ttl(&ban_key(proxy, subject)).await.ok().flatten()
}

/// Record a failed authentication and ban the caller once the threshold is hit.
pub async fn record_auth_failure(state: &State, proxy: &ProxyKubeApi, subject: &str) {
    let Some(config) = security_config(proxy) else {
        return;
    };
    if !config.fail2login_enabled() {
        return;
    }

    let failures = match state
        .incr_with_ttl(&failure_key(proxy, subject), FAILURE_WINDOW_SECONDS)
        .await
    {
        Ok(failures) => failures,
        Err(err) => {
            warn!(%err, "could not record the failed authentication");
            return;
        }
    };

    // Saturate rather than silently wrap: an absurd failure count still maps to
    // the largest configured ban tier.
    let failures_u32 = u32::try_from(failures).unwrap_or(u32::MAX);
    let Some(ban_duration) = config.ban_duration_for(failures_u32) else {
        debug!(failures, subject, "failed authentication recorded");
        return;
    };

    warn!(
        failures,
        subject, ban_duration, "banning the client after too many failed authentications"
    );
    if let Err(err) = state
        .set_flag(&ban_key(proxy, subject), u64::from(ban_duration))
        .await
    {
        warn!(%err, "could not apply the ban");
    }
}

/// Forget the failure history of a caller that just authenticated.
pub async fn clear_auth_failures(state: &State, proxy: &ProxyKubeApi, subject: &str) {
    let Some(config) = security_config(proxy) else {
        return;
    };
    if !config.fail2login_enabled() {
        return;
    }
    if let Err(err) = state.delete_key(&failure_key(proxy, subject)).await {
        debug!(%err, "could not clear the failure counter");
    }
}

/// Count this request and report whether it went over the caller's budget.
///
/// Like `is_banned`, a Redis failure fails open.
pub async fn check_rate_limit(
    state: &State,
    proxy: &ProxyKubeApi,
    subject: &str,
    groups: &[String],
) -> Option<Throttled> {
    let config = security_config(proxy)?;
    let limit = config.requests_per_minute(groups)?;

    let used = match state
        .incr_with_ttl(&rate_limit_key(proxy, subject), RATE_LIMIT_WINDOW_SECONDS)
        .await
    {
        Ok(used) => used,
        Err(err) => {
            if fail_closed() {
                warn!(%err, "rate limit counter unavailable; failing closed (refusing the request)");
                return Some(Throttled::RateLimited {
                    limit,
                    retry_after: RATE_LIMIT_WINDOW_SECONDS as u64,
                });
            }
            warn!(%err, "could not read the rate limit counter, letting the request through");
            return None;
        }
    };

    if used > u64::from(limit) {
        warn!(subject, used, limit, "rate limit exceeded");
        return Some(Throttled::RateLimited {
            limit,
            retry_after: RATE_LIMIT_WINDOW_SECONDS as u64,
        });
    }
    None
}

/// Throttling for endpoints that are not scoped to a cluster.
///
/// `/api/v1/clusters` (the dashboard listing) authenticates every caller, so an
/// anonymous request still costs one outbound `/userinfo` call to the provider
/// before it can be refused. Everything above keys on a `ProxyKubeApi` and reads
/// its `SecurityConfiguration`; there is no proxy here, so the budget comes from
/// the environment instead and the Redis keys carry a fixed scope.
///
/// **Both limits are off by default, deliberately.** Behind an ingress that has
/// not had `TRUSTED_PROXY_COUNT` configured, every caller collapses to the
/// ingress address — so a default-on ban would let one bad client lock out
/// everybody, which is the same hazard `redirect()` documents for its own
/// pre-authentication half. An operator who has configured the trusted-proxy
/// depth (or terminates TLS directly) can turn these on and get a real budget;
/// one who has not is no worse off than before.
pub mod unscoped {
    use super::{FAILURE_WINDOW_SECONDS, RATE_LIMIT_WINDOW_SECONDS, Throttled, fail_closed};
    use common::State;
    use tracing::{debug, warn};

    /// Key scope for throttling with no cluster to name. Distinct from any
    /// `to_path()` value, which is always `namespace/name`.
    const SCOPE: &str = "_unscoped";

    fn rate_limit_key(subject: &str) -> String {
        format!("proxyk8sauth:ratelimit:{SCOPE}:{subject}")
    }

    fn failure_key(subject: &str) -> String {
        format!("proxyk8sauth:fail2login:{SCOPE}:{subject}")
    }

    fn ban_key(subject: &str) -> String {
        format!("proxyk8sauth:ban:{SCOPE}:{subject}")
    }

    /// Read a `u32` knob, treating absent/unparsable/0 as "disabled".
    fn limit_from_env(name: &str) -> Option<u32> {
        std::env::var(name)
            .ok()?
            .trim()
            .parse::<u32>()
            .ok()
            .filter(|value| *value > 0)
    }

    /// Requests per minute allowed per caller, from
    /// `UNSCOPED_RATE_LIMIT_PER_MINUTE`. Unset or `0` disables the limit.
    fn rate_limit() -> Option<u32> {
        limit_from_env("UNSCOPED_RATE_LIMIT_PER_MINUTE")
    }

    /// Failed authentications tolerated before a ban, from
    /// `UNSCOPED_MAX_FAILED_LOGINS`. Unset or `0` disables fail2login.
    fn max_failed_logins() -> Option<u32> {
        limit_from_env("UNSCOPED_MAX_FAILED_LOGINS")
    }

    /// Ban length in seconds, from `UNSCOPED_BAN_DURATION_SECONDS` (default 300).
    fn ban_duration() -> u64 {
        limit_from_env("UNSCOPED_BAN_DURATION_SECONDS").map_or(300, u64::from)
    }

    /// Whether `subject` is currently banned on the unscoped surface.
    ///
    /// Fails open on a Redis error for the same reason as the cluster-scoped
    /// check: a counter store outage must not become a full outage.
    pub async fn is_banned(state: &State, subject: &str) -> bool {
        if max_failed_logins().is_none() {
            return false;
        }
        match state.key_exists(&ban_key(subject)).await {
            Ok(banned) => banned,
            Err(err) => {
                if fail_closed() {
                    warn!(%err, "ban state unavailable; failing closed (refusing the request)");
                    true
                } else {
                    warn!(%err, "could not read the ban state, letting the request through");
                    false
                }
            }
        }
    }

    /// Remaining ban duration, for the `Retry-After` header.
    pub async fn ban_retry_after(state: &State, subject: &str) -> Option<u64> {
        state.key_ttl(&ban_key(subject)).await.ok().flatten()
    }

    /// Record a failed authentication and ban once the threshold is reached.
    pub async fn record_auth_failure(state: &State, subject: &str) {
        let Some(max_failures) = max_failed_logins() else {
            return;
        };

        let failures = match state
            .incr_with_ttl(&failure_key(subject), FAILURE_WINDOW_SECONDS)
            .await
        {
            Ok(failures) => failures,
            Err(err) => {
                warn!(%err, "could not record the failed authentication");
                return;
            }
        };

        if u32::try_from(failures).unwrap_or(u32::MAX) < max_failures {
            debug!(failures, subject, "failed authentication recorded");
            return;
        }

        let duration = ban_duration();
        warn!(
            failures,
            subject, duration, "banning the client after too many failed authentications"
        );
        if let Err(err) = state.set_flag(&ban_key(subject), duration).await {
            warn!(%err, "could not apply the ban");
        }
    }

    /// Forget the failure history of a caller that just authenticated.
    pub async fn clear_auth_failures(state: &State, subject: &str) {
        if max_failed_logins().is_none() {
            return;
        }
        if let Err(err) = state.delete_key(&failure_key(subject)).await {
            debug!(%err, "could not clear the failure counter");
        }
    }

    /// Count this request and report whether it went over the caller's budget.
    pub async fn check_rate_limit(state: &State, subject: &str) -> Option<Throttled> {
        let limit = rate_limit()?;

        let used = match state
            .incr_with_ttl(&rate_limit_key(subject), RATE_LIMIT_WINDOW_SECONDS)
            .await
        {
            Ok(used) => used,
            Err(err) => {
                if fail_closed() {
                    warn!(%err, "rate limit counter unavailable; failing closed");
                    return Some(Throttled::RateLimited {
                        limit,
                        retry_after: RATE_LIMIT_WINDOW_SECONDS as u64,
                    });
                }
                warn!(%err, "could not read the rate limit counter, letting the request through");
                return None;
            }
        };

        if used > u64::from(limit) {
            warn!(subject, used, limit, "rate limit exceeded");
            return Some(Throttled::RateLimited {
                limit,
                retry_after: RATE_LIMIT_WINDOW_SECONDS as u64,
            });
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crd::ProxyKubeApiSpec;
    use crd::certificate::CertSource;
    use crd::service::Service;

    fn proxy() -> ProxyKubeApi {
        let mut proxy = ProxyKubeApi::new(
            "local",
            ProxyKubeApiSpec {
                enabled: true,
                cert: CertSource::Insecure(true),
                client_cert: None,
                service: Service::ExternalService {
                    url: "https://cluster.example.com".to_string(),
                },
                auth_config: None,
                security_config: None,
                expose_via_dashboard: false,
                dashboard_group: None,
                proxy_group: None,
                virtual_apis: Vec::new(),
            },
        );
        proxy.metadata.namespace = Some("default".to_string());
        proxy
    }

    #[test]
    fn keys_are_namespaced_per_cluster_and_subject() {
        let proxy = proxy();
        assert_eq!(
            rate_limit_key(&proxy, "alice"),
            "proxyk8sauth:ratelimit:default/local:alice"
        );
        assert_eq!(
            failure_key(&proxy, "203.0.113.7"),
            "proxyk8sauth:fail2login:default/local:203.0.113.7"
        );
        assert_eq!(
            ban_key(&proxy, "203.0.113.7"),
            "proxyk8sauth:ban:default/local:203.0.113.7"
        );
    }

    #[test]
    fn throttled_reports_a_retry_delay() {
        let rate_limited = Throttled::RateLimited {
            limit: 60,
            retry_after: 60,
        };
        assert_eq!(rate_limited.retry_after(), Some(60));
        assert!(rate_limited.message().contains("60 requests per minute"));

        assert_eq!(Throttled::Banned { retry_after: None }.retry_after(), None);
        assert_eq!(
            Throttled::Banned {
                retry_after: Some(300)
            }
            .retry_after(),
            Some(300)
        );
    }
}
