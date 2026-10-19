//! Indirect checks, read with the admin kubeconfig: the back's Prometheus
//! metrics through the apiserver's service proxy, and its audit log lines.

use crate::{
    Bench,
    bench::{BACK_SERVICE, STACK_NS},
};

/// The back's `/management/metrics`, scraped as the admin.
pub fn metrics() -> String {
    let path = format!(
        "/api/v1/namespaces/{STACK_NS}/services/http:{BACK_SERVICE}:http/proxy/management/metrics"
    );
    let out = Bench::get().admin(&["get", "--raw", &path]);
    assert!(
        out.status.success(),
        "scrape metrics: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Sum of the samples of `name` whose labels include every `labels` pair.
pub fn metric_value(text: &str, name: &str, labels: &[(&str, &str)]) -> f64 {
    text.lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| {
            let (series, value) = line.rsplit_once(' ')?;
            let (metric, rest) = series.split_once('{').unwrap_or((series, "}"));
            (metric == name).then_some(())?;
            labels
                .iter()
                .all(|(k, v)| rest.contains(&format!("{k}=\"{v}\"")))
                .then(|| value.parse::<f64>().ok())?
        })
        .sum()
}

/// How much `name{labels}` moved while `action` ran: an assertion on the
/// difference, never on an absolute value (other tests share the counter).
pub async fn metric_delta<F: Future>(
    name: &str,
    labels: &[(&str, &str)],
    action: F,
) -> (f64, F::Output) {
    let before = metric_value(&metrics(), name, labels);
    let result = action.await;
    let after = metric_value(&metrics(), name, labels);
    (after - before, result)
}

/// The back's audit lines (RFC 0008: one JSON object with `audit_schema`)
/// that contain `needle`.
pub fn audit_lines(needle: &str) -> Vec<serde_json::Value> {
    let out = Bench::get().admin(&[
        "-n",
        STACK_NS,
        "logs",
        &format!("deployment/{BACK_SERVICE}"),
        "--tail=-1",
    ]);
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|line| line.contains(needle))
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|event| event.get("audit_schema").is_some())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::metric_value;

    #[test]
    fn sums_matching_series_only() {
        let text = "# HELP x\n\
            proxyauth_policy_decisions_total{cluster=\"a/b\",decision=\"denied\",policy=\"verb\"} 2\n\
            proxyauth_policy_decisions_total{cluster=\"a/c\",decision=\"denied\",policy=\"verb\"} 3\n\
            proxyauth_policy_decisions_total{cluster=\"a/b\",decision=\"allowed\",policy=\"port\"} 7\n\
            other_total 1\n";
        let name = "proxyauth_policy_decisions_total";
        assert_eq!(metric_value(text, name, &[("decision", "denied")]), 5.0);
        assert_eq!(
            metric_value(text, name, &[("cluster", "a/b"), ("policy", "verb")]),
            2.0
        );
        assert_eq!(metric_value(text, "other_total", &[]), 1.0);
        assert_eq!(metric_value(text, "missing", &[]), 0.0);
    }
}
