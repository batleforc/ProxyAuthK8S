use std::time::Duration;

/// Parse a Kubernetes / Go style duration such as `32s`, `1m30s`, `500ms` or `1.5h`.
///
/// The apiserver sends `timeout=32s` on watch requests, so a plain
/// `parse::<u64>()` silently fails and the timeout is never applied. A bare
/// integer (no unit) is still accepted and interpreted as a number of seconds,
/// which is what `timeoutSeconds` uses.
///
/// Returns `None` for anything that is not a well formed, non negative duration.
pub fn parse_kube_duration(value: &str) -> Option<Duration> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }

    // A bare integer means seconds (`timeoutSeconds` style).
    if value.chars().all(|c| c.is_ascii_digit()) {
        return value.parse::<u64>().ok().map(Duration::from_secs);
    }

    let mut total_nanos: f64 = 0.0;
    let mut rest = value;

    while !rest.is_empty() {
        let number_len = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(rest.len());
        if number_len == 0 {
            return None;
        }
        let number: f64 = rest[..number_len].parse().ok()?;
        if !number.is_finite() {
            return None;
        }
        rest = &rest[number_len..];

        // The unit runs until the next digit (or the end of the input).
        let unit_len = rest
            .find(|c: char| c.is_ascii_digit())
            .unwrap_or(rest.len());
        if unit_len == 0 {
            return None;
        }
        let (unit, remainder) = rest.split_at(unit_len);
        rest = remainder;

        let unit_nanos: f64 = match unit {
            "ns" => 1.0,
            "us" | "µs" | "\u{03bc}s" => 1_000.0,
            "ms" => 1_000_000.0,
            "s" => 1_000_000_000.0,
            "m" => 60.0 * 1_000_000_000.0,
            "h" => 3_600.0 * 1_000_000_000.0,
            _ => return None,
        };
        total_nanos += number * unit_nanos;
    }

    // `total_nanos` is always >= 0 (the grammar has no sign), so only the upper
    // bound needs checking before the cast.
    if total_nanos > u64::MAX as f64 {
        return None;
    }
    Some(Duration::from_nanos(total_nanos as u64))
}

/// Extract the `timeout` query parameter and parse it as a Kubernetes duration.
pub fn extract_timeout_from_query(query_string: &str) -> Option<Duration> {
    query_string
        .split('&')
        .find_map(|param| param.strip_prefix("timeout="))
        .and_then(parse_kube_duration)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bare_integer_as_seconds() {
        assert_eq!(parse_kube_duration("32"), Some(Duration::from_secs(32)));
        assert_eq!(parse_kube_duration("0"), Some(Duration::ZERO));
    }

    #[test]
    fn parses_single_unit() {
        assert_eq!(parse_kube_duration("32s"), Some(Duration::from_secs(32)));
        assert_eq!(parse_kube_duration("5m"), Some(Duration::from_secs(300)));
        assert_eq!(parse_kube_duration("2h"), Some(Duration::from_secs(7200)));
        assert_eq!(
            parse_kube_duration("500ms"),
            Some(Duration::from_millis(500))
        );
        assert_eq!(parse_kube_duration("10us"), Some(Duration::from_micros(10)));
        assert_eq!(parse_kube_duration("10µs"), Some(Duration::from_micros(10)));
        assert_eq!(parse_kube_duration("7ns"), Some(Duration::from_nanos(7)));
    }

    #[test]
    fn parses_compound_units() {
        assert_eq!(parse_kube_duration("1m30s"), Some(Duration::from_secs(90)));
        assert_eq!(
            parse_kube_duration("2h45m10s"),
            Some(Duration::from_secs(2 * 3600 + 45 * 60 + 10))
        );
    }

    #[test]
    fn parses_fractional_values() {
        assert_eq!(parse_kube_duration("1.5h"), Some(Duration::from_secs(5400)));
        assert_eq!(
            parse_kube_duration("0.5s"),
            Some(Duration::from_millis(500))
        );
    }

    #[test]
    fn rejects_invalid_input() {
        assert_eq!(parse_kube_duration(""), None);
        assert_eq!(parse_kube_duration("   "), None);
        assert_eq!(parse_kube_duration("abc"), None);
        assert_eq!(parse_kube_duration("32y"), None);
        assert_eq!(parse_kube_duration("-5s"), None);
        assert_eq!(parse_kube_duration("s"), None);
        assert_eq!(parse_kube_duration("10s5"), None);
    }

    #[test]
    fn extracts_timeout_from_query_string() {
        assert_eq!(
            extract_timeout_from_query("watch=true&timeout=32s&allowWatchBookmarks=true"),
            Some(Duration::from_secs(32))
        );
        assert_eq!(
            extract_timeout_from_query("timeout=1m30s"),
            Some(Duration::from_secs(90))
        );
        assert_eq!(extract_timeout_from_query("watch=true"), None);
        // `timeoutSeconds` must not be mistaken for `timeout`.
        assert_eq!(extract_timeout_from_query("timeoutSeconds=30"), None);
    }
}
