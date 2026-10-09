//! Prometheus pull endpoint (`/management/metrics`) and the HTTP server
//! request-duration instrument it exposes.

use actix_web::{App, http::StatusCode, test, web};
use api::{
    base::health,
    metrics::{
        HTTP_SERVER_REQUEST_DURATION, HttpMetrics, PROMETHEUS_CONTENT_TYPE, PrometheusRegistry,
        prometheus_metrics,
    },
};
use opentelemetry::metrics::MeterProvider as _;
use opentelemetry_sdk::metrics::SdkMeterProvider;

/// A meter provider exporting into a fresh Prometheus registry, as the
/// `trace` crate builds it when `METRICS_PROMETHEUS_ENABLED` is set.
fn prometheus_provider() -> (SdkMeterProvider, prometheus::Registry) {
    let registry = prometheus::Registry::new();
    let exporter = opentelemetry_prometheus::exporter()
        .with_registry(registry.clone())
        .build()
        .expect("register the Prometheus exporter");
    let provider = SdkMeterProvider::builder().with_reader(exporter).build();
    (provider, registry)
}

macro_rules! metrics_app {
    ($registry:expr_2021, $http_metrics:expr_2021) => {
        test::init_service(
            App::new()
                .wrap($http_metrics)
                .app_data(web::Data::new($registry))
                .service(
                    web::scope("/management")
                        .service(health)
                        .service(prometheus_metrics),
                ),
        )
        .await
    };
}

#[actix_web::test]
async fn the_endpoint_serves_the_http_server_metrics_when_enabled() {
    let (provider, registry) = prometheus_provider();
    let app = metrics_app!(
        PrometheusRegistry(Some(registry)),
        HttpMetrics::new(&provider.meter("proxyauthk8s"))
    );

    let res = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/management/health")
            .to_request(),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);

    let res = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/management/metrics")
            .to_request(),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some(PROMETHEUS_CONTENT_TYPE)
    );
    let body = String::from_utf8(test::read_body(res).await.to_vec()).unwrap();

    let name = HTTP_SERVER_REQUEST_DURATION.replace('.', "_") + "_seconds";
    assert!(body.contains(&format!("# HELP {name} ")), "{body}");
    assert!(body.contains(&format!("# TYPE {name} histogram")), "{body}");
    let health_count = body
        .lines()
        .find(|line| {
            line.starts_with(&format!("{name}_count{{"))
                && line.contains(r#"http_route="/management/health""#)
        })
        .unwrap_or_else(|| panic!("no count series for the health route:\n{body}"));
    assert!(
        health_count.contains(r#"http_request_method="GET""#),
        "{health_count}"
    );
    assert!(
        health_count.contains(r#"http_response_status_code="200""#),
        "{health_count}"
    );
    assert!(health_count.ends_with(" 1"), "{health_count}");
}

#[actix_web::test]
async fn the_endpoint_is_404_when_disabled() {
    let (provider, _registry) = prometheus_provider();
    let app = metrics_app!(
        PrometheusRegistry(None),
        HttpMetrics::new(&provider.meter("proxyauthk8s"))
    );
    let res = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/management/metrics")
            .to_request(),
    )
    .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[actix_web::test]
async fn the_endpoint_is_404_without_registry_app_data() {
    let app = test::init_service(
        App::new().service(web::scope("/management").service(prometheus_metrics)),
    )
    .await;
    let res = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/management/metrics")
            .to_request(),
    )
    .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}
