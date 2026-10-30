//! Guard (RFC 0015 §6.3): every 2xx response of the OpenAPI document declares
//! its body, or is listed in `NO_BODY` with the reason. A response without a
//! body reaches the generated clients (HeyApi, the CLI's) as `unknown`.

#![expect(
    clippy::expect_used,
    reason = "test helpers: a panic is a failed test, with its message"
)]

use api::gen_openapi;

/// `(method, path, status)` answered without a declared body, and why.
const NO_BODY: &[(&str, &str, &str, &str)] = &[
    (
        "GET",
        "/management/health",
        "200",
        "liveness answers an empty 200",
    ),
    (
        "GET",
        "/clusters/{ns}/{cluster}/oauth/jwks",
        "200",
        "relays the IdP's JWKS as received; declaring it would change both generated clients for an endpoint none of them calls",
    ),
    (
        "GET",
        "/clusters/{ns}/{cluster}/{path}",
        "200",
        "relays the apiserver's answer, whatever the resource",
    ),
    (
        "POST",
        "/clusters/{ns}/{cluster}/{path}",
        "200",
        "relays the apiserver's answer",
    ),
    (
        "PUT",
        "/clusters/{ns}/{cluster}/{path}",
        "200",
        "relays the apiserver's answer",
    ),
    (
        "PATCH",
        "/clusters/{ns}/{cluster}/{path}",
        "200",
        "relays the apiserver's answer",
    ),
    (
        "DELETE",
        "/clusters/{ns}/{cluster}/{path}",
        "200",
        "relays the apiserver's answer",
    ),
];

/// Every 2xx response but 204, as `(method, path, status, has_body)`.
fn success_responses(document: &serde_json::Value) -> Vec<(String, String, String, bool)> {
    let mut found = Vec::new();
    for (path, item) in document["paths"].as_object().expect("paths") {
        for (method, operation) in item.as_object().expect("path item") {
            let Some(responses) = operation.get("responses").and_then(|r| r.as_object()) else {
                continue;
            };
            for (status, response) in responses {
                if status.starts_with('2') && status != "204" {
                    let has_body = response
                        .get("content")
                        .and_then(|content| content.as_object())
                        .is_some_and(|content| !content.is_empty());
                    found.push((
                        method.to_uppercase(),
                        path.clone(),
                        status.clone(),
                        has_body,
                    ));
                }
            }
        }
    }
    found
}

/// The problems the guard reports: a response without a body and without an
/// exemption, or an exemption that no longer matches a bodiless response.
fn violations(responses: &[(String, String, String, bool)]) -> Vec<String> {
    let exempt = |method: &str, path: &str, status: &str| {
        NO_BODY
            .iter()
            .any(|(m, p, s, _)| *m == method && *p == path && *s == status)
    };
    let mut problems: Vec<String> = responses
        .iter()
        .filter(|(method, path, status, has_body)| !has_body && !exempt(method, path, status))
        .map(|(method, path, status, _)| {
            format!(
                "openapi_contract: {method} {path} {status} declares no body \
                 (add body = T to its #[utoipa::path], or list it in NO_BODY with a reason)"
            )
        })
        .collect();
    for (method, path, status, _) in NO_BODY {
        let still_bodiless = responses
            .iter()
            .any(|(m, p, s, has_body)| m == method && p == path && s == status && !has_body);
        if !still_bodiless {
            problems.push(format!(
                "openapi_contract: NO_BODY lists {method} {path} {status}, which is gone or now has a body; drop the entry"
            ));
        }
    }
    problems
}

#[test]
fn every_success_response_declares_its_body() {
    let document = serde_json::to_value(gen_openapi()).expect("the document serializes");
    let problems = violations(&success_responses(&document));
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// The guard can fail: a bodiless response it was not told about is reported,
/// and so is an exemption left behind.
#[test]
fn the_guard_reports_a_bodiless_response_and_a_stale_exemption() {
    let mut responses: Vec<_> = NO_BODY
        .iter()
        .map(|(m, p, s, _)| (m.to_string(), p.to_string(), s.to_string(), false))
        .collect();
    responses.push(("POST".into(), "/api/v1/new".into(), "201".into(), false));
    let problems = violations(&responses);
    assert_eq!(problems.len(), 1);
    assert!(problems[0].contains("POST /api/v1/new 201 declares no body"));

    responses.retain(|(_, path, _, _)| path != "/management/health");
    let problems = violations(&responses);
    assert!(
        problems
            .iter()
            .any(|p| p.contains("NO_BODY lists GET /management/health"))
    );
}
