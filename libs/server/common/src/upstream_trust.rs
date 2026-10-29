//! The trust probe's last word on each `FrontProxy` cluster (RFC 0014 §5.4):
//! written by the probe in the `api` process, read by the controller of the
//! same process when it reconciles. In memory only: a restart probes again.

use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};
use std::time::{Duration, Instant, SystemTime};

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

/// When a result was seen, since when its verdict holds, and the result.
type Entry = (Instant, SystemTime, Trust);

static RESULTS: LazyLock<RwLock<HashMap<String, Entry>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Record the probe's result for the cluster `ns/name`. The verdict's start
/// is kept while the verdict (not its detail) stays the same.
pub fn record(cluster: &str, trust: Trust) {
    if let Ok(mut results) = RESULTS.write() {
        let since = results
            .get(cluster)
            .filter(|(at, _, last)| {
                at.elapsed() < MAX_AGE
                    && std::mem::discriminant(last) == std::mem::discriminant(&trust)
            })
            .map_or_else(SystemTime::now, |(_, since, _)| *since);
        results.insert(cluster.to_string(), (Instant::now(), since, trust));
    }
}

/// The last result for `ns/name`, if recent, with when its verdict began.
#[must_use]
pub fn get(cluster: &str) -> Option<(Trust, SystemTime)> {
    let results = RESULTS.read().ok()?;
    let (at, since, trust) = results.get(cluster)?;
    (at.elapsed() < MAX_AGE).then(|| (trust.clone(), *since))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_result_is_kept_per_cluster() {
        record("t/a", Trust::Rejected);
        record("t/b", Trust::Trusted);
        assert_eq!(get("t/a").map(|(t, _)| t), Some(Trust::Rejected));
        assert_eq!(get("t/b").map(|(t, _)| t), Some(Trust::Trusted));
        assert_eq!(get("t/none"), None);
    }

    #[test]
    fn the_verdict_start_moves_only_when_the_verdict_changes() {
        record("t/c", Trust::Unknown("timeout".into()));
        let first = get("t/c").map(|(_, since)| since);
        record("t/c", Trust::Unknown("503".into()));
        assert_eq!(get("t/c").map(|(_, since)| since), first);
        std::thread::sleep(Duration::from_millis(5));
        record("t/c", Trust::Trusted);
        assert!(get("t/c").map(|(_, since)| since) > first);
    }
}
