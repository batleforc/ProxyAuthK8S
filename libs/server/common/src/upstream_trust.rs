//! The trust probe's last word on each `FrontProxy` cluster (RFC 0014 §5.4):
//! written by the probe in the `api` process, read by the controller of the
//! same process when it reconciles. In memory only: a restart probes again.

use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};
use std::time::{Duration, Instant};

/// What the apiserver made of the identity the probe asserted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Trust {
    /// It saw that identity, group and `extra` included.
    Trusted,
    /// It saw another one: the certificate is taken for itself
    /// (`--client-ca-file`), or a header is not read.
    NotHonoured(String),
    /// It answered `401`: CA or `allowed-names`.
    Rejected,
    /// Transport error, `5xx`, no agent, timeout: no conclusion.
    Unknown(String),
}

/// A result older than this is no result: the controller then reports none.
pub const MAX_AGE: Duration = Duration::from_secs(15 * 60);

static RESULTS: LazyLock<RwLock<HashMap<String, (Instant, Trust)>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Record the probe's result for the cluster `ns/name`.
pub fn record(cluster: &str, trust: Trust) {
    if let Ok(mut results) = RESULTS.write() {
        results.insert(cluster.to_string(), (Instant::now(), trust));
    }
}

/// The last result for `ns/name`, if recent.
#[must_use]
pub fn get(cluster: &str) -> Option<Trust> {
    let results = RESULTS.read().ok()?;
    let (at, trust) = results.get(cluster)?;
    (at.elapsed() < MAX_AGE).then(|| trust.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_result_is_kept_per_cluster() {
        record("t/a", Trust::Rejected);
        record("t/b", Trust::Trusted);
        assert_eq!(get("t/a"), Some(Trust::Rejected));
        assert_eq!(get("t/b"), Some(Trust::Trusted));
        assert_eq!(get("t/none"), None);
    }
}
