//! Classification of the requests that reach the loopback listener.

/// Outcome of parsing a request that hit the loopback listener.
pub(super) enum CallbackResult {
    /// The OIDC provider redirected back with an authorization `code`+`state`.
    Success { code: String, state: String },
    /// The provider redirected back with an `error` (RFC 6749 §4.1.2.1).
    ProviderError {
        error: String,
        description: Option<String>,
    },
}

/// Classify a request target such as
/// `/auth/callback/ns/cluster?code=..&state=..` (success) or
/// `/auth/callback/ns/cluster?error=access_denied&error_description=..` (the
/// provider rejected the request). Returns `None` for anything that is neither
/// (favicon fetches, health probes, …) so the caller keeps waiting.
pub(super) fn parse_callback(target: &str) -> Option<CallbackResult> {
    let url = reqwest::Url::parse(&format!("http://localhost{target}")).ok()?;
    let mut code = None;
    let mut state = None;
    let mut error = None;
    let mut description = None;
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "code" => code = Some(value.into_owned()),
            "state" => state = Some(value.into_owned()),
            "error" => error = Some(value.into_owned()),
            "error_description" => description = Some(value.into_owned()),
            _ => {}
        }
    }
    if let Some(error) = error {
        return Some(CallbackResult::ProviderError { error, description });
    }
    Some(CallbackResult::Success {
        code: code?,
        state: state?,
    })
}

#[cfg(test)]
mod tests {
    use super::{CallbackResult, parse_callback};

    #[test]
    fn parses_code_and_state() {
        let target = "/auth/callback/default/local?code=abc&state=xyz";
        match parse_callback(target) {
            Some(CallbackResult::Success { code, state }) => {
                assert_eq!(code, "abc");
                assert_eq!(state, "xyz");
            }
            _ => panic!("expected a successful callback"),
        }
    }

    #[test]
    fn parses_provider_error() {
        let target =
            "/auth/callback/default/local?error=access_denied&error_description=user%20said%20no";
        match parse_callback(target) {
            Some(CallbackResult::ProviderError { error, description }) => {
                assert_eq!(error, "access_denied");
                assert_eq!(description.as_deref(), Some("user said no"));
            }
            _ => panic!("expected a provider error"),
        }
    }

    #[test]
    fn error_takes_precedence_over_partial_code() {
        // A stray `code` with an `error` must still be treated as a failure.
        let target = "/auth/callback/default/local?error=invalid_request&code=abc";
        assert!(matches!(
            parse_callback(target),
            Some(CallbackResult::ProviderError { .. })
        ));
    }

    #[test]
    fn ignores_non_callback_requests() {
        // Neither code+state nor error -> keep waiting (favicon, health probes).
        assert!(parse_callback("/favicon.ico").is_none());
        assert!(parse_callback("/auth/callback/default/local?state=only").is_none());
    }
}
