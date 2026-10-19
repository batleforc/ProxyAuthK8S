//! RFC 0002 §6.P on the real front image, served by nginx behind the Ingress
//! with the variables the chart sets.

use integration::Bench;

/// Point 8: the security headers of the image, and a `connect-src` that
/// names the API URL the container was given.
#[tokio::test]
async fn p8_the_front_image_serves_its_security_headers() {
    let bench = Bench::get();
    let response = bench
        .http()
        .get(bench.base_url())
        .send()
        .await
        .expect("GET /");
    assert_eq!(response.status(), 200);
    let header = |name: &str| {
        response
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string()
    };
    assert_eq!(header("x-content-type-options"), "nosniff");
    assert_eq!(header("referrer-policy"), "no-referrer");
    let csp = header("content-security-policy");
    let connect = csp
        .split(';')
        .map(str::trim)
        .find(|d| d.starts_with("connect-src"))
        .unwrap_or_else(|| panic!("no connect-src in {csp:?}"));
    assert!(
        connect.split_whitespace().any(|s| s == bench.base_url()),
        "{connect:?} does not name {}",
        bench.base_url()
    );
    assert!(csp.contains("script-src 'self' 'sha256-"), "{csp:?}");
}
