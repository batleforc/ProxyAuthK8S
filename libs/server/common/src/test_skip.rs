//! How a test that needs a real Redis says it did not run (RFC 0015 §4.2): a
//! skip is never silent.

use std::io::Write;

/// Report that the calling test cannot reach the Redis at `url`, then let it
/// return. The URL's credentials are left out of the message.
///
/// Prints `SKIPPED: <test> — <reason>` on stderr and appends the same line to
/// the file named by `TEST_SKIP_LOG`, which `task test:unit` prints at the end
/// (nextest hides a passing test's stderr). Under `REQUIRE_TEST_REDIS` the
/// skip is a failure instead.
///
/// # Panics
///
/// When `REQUIRE_TEST_REDIS` is set: a run that promised Redis must not pass
/// without it.
pub fn redis_unavailable(url: &str, error: &str) {
    let reason = format!(
        "test Redis at {} is unreachable ({}); set TEST_REDIS_URL to override",
        without_credentials(url),
        error.replace('\n', " ")
    );
    let test = std::thread::current()
        .name()
        .unwrap_or("<unnamed test>")
        .to_string();
    assert!(
        std::env::var_os("REQUIRE_TEST_REDIS").is_none(),
        "REQUIRE_TEST_REDIS is set but {test} cannot run: {reason}"
    );
    let line = format!("SKIPPED: {test} — {reason}");
    eprintln!("{line}");
    let Some(path) = std::env::var_os("TEST_SKIP_LOG") else {
        return;
    };
    let written = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        // One write of the whole line: tests in parallel processes share the
        // file, and O_APPEND keeps a single small write whole.
        .and_then(|mut log| log.write_all(format!("{line}\n").as_bytes()));
    if let Err(error) = written {
        eprintln!("could not append to TEST_SKIP_LOG: {error}");
    }
}

/// `url` with its `user:password@` part replaced by `***@`.
fn without_credentials(url: &str) -> String {
    match (url.find("://"), url.rfind('@')) {
        (Some(scheme), Some(at)) if at > scheme => {
            format!(
                "{}***{}",
                url.get(..scheme.saturating_add(3)).unwrap_or_default(),
                url.get(at..).unwrap_or_default()
            )
        }
        _ => url.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::without_credentials;

    #[test]
    fn credentials_are_left_out_of_the_message() {
        assert_eq!(
            without_credentials("redis://:s3cret%23@localhost:6379"),
            "redis://***@localhost:6379"
        );
        assert_eq!(
            without_credentials("redis://user:pw@host:1/0"),
            "redis://***@host:1/0"
        );
        assert_eq!(
            without_credentials("redis://127.0.0.1:6379"),
            "redis://127.0.0.1:6379"
        );
    }
}
