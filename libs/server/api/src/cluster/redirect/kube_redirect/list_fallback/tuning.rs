//! Process-global tuning knobs for the filtered `LIST` path.
//!
//! Environment variables never change after startup, so each is parsed once
//! into a `OnceLock` rather than re-read from `std::env` on a per-request hot
//! path.

use std::sync::OnceLock;

/// Upper bound on concurrent `SelfSubjectAccessReview` calls for one LIST.
const DEFAULT_CONCURRENCY: usize = 16;

/// Environment variables are process-global and never change after startup,
/// so each of these knobs is parsed once and cached rather than re-read from
/// `std::env` on every call in what can be a per-request hot path.
pub(super) fn concurrency() -> usize {
    static CONCURRENCY: OnceLock<usize> = OnceLock::new();
    *CONCURRENCY.get_or_init(|| {
        std::env::var("PROXY_VIRTUAL_LIST_FALLBACK_CONCURRENCY")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_CONCURRENCY)
    })
}

/// How long a resolved allow-set is trusted before being recomputed.
///
/// Short enough that a revoked grant stops being honoured soon after; long
/// enough that a polling dashboard does not re-run a rules review (or worse,
/// per-item access reviews) on every refresh. `0` disables caching outright.
const DEFAULT_CACHE_TTL_SECONDS: u64 = 30;

pub(super) fn cache_ttl_seconds() -> u64 {
    static TTL: OnceLock<u64> = OnceLock::new();
    *TTL.get_or_init(|| {
        std::env::var("PROXY_VIRTUAL_LIST_FALLBACK_CACHE_TTL_SECONDS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(DEFAULT_CACHE_TTL_SECONDS)
    })
}

/// Upper bound on a single upstream call this module makes directly
/// (`SelfSubjectRulesReview`, `SelfSubjectAccessReview`, or the privileged
/// namespace list). Applied per request via `RequestBuilder::timeout`, not on
/// the shared client — the shared client sets none, precisely so a long-lived
/// watch (built and used elsewhere, never through this module) is unaffected.
/// A cache-miss resolution here runs behind a per-key lock (see
/// [`resolve_outcome_for`]), so without this a stalled upstream call would
/// queue every concurrent caller sharing that key indefinitely instead of
/// just itself.
const DEFAULT_UPSTREAM_TIMEOUT_SECONDS: u64 = 30;

pub(super) fn upstream_timeout() -> std::time::Duration {
    static TIMEOUT: OnceLock<std::time::Duration> = OnceLock::new();
    *TIMEOUT.get_or_init(|| {
        let seconds = std::env::var("PROXY_VIRTUAL_LIST_FALLBACK_TIMEOUT_SECONDS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_UPSTREAM_TIMEOUT_SECONDS);
        std::time::Duration::from_secs(seconds)
    })
}
