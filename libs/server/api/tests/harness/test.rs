//! `actix_web::test`'s shape over the axum router (RFC 0007 §6.5): the tests
//! ported from actix keep their requests and assertions, while each call goes
//! through `tower::ServiceExt::oneshot` on the real routes.

use axum::{
    Router,
    body::{Body, Bytes},
    http::{HeaderName, HeaderValue, Method, Request},
    response::Response,
};
use http_body_util::BodyExt as _;
use tower::ServiceExt as _;

/// A request under construction, as `actix_web::test::TestRequest` builds it.
pub struct TestRequest {
    builder: axum::http::request::Builder,
    body: Body,
}

impl TestRequest {
    fn with_method(method: Method) -> Self {
        Self {
            builder: Request::builder().method(method).uri("/"),
            body: Body::empty(),
        }
    }

    pub fn default() -> Self {
        Self::get()
    }

    pub fn get() -> Self {
        Self::with_method(Method::GET)
    }

    pub fn post() -> Self {
        Self::with_method(Method::POST)
    }

    pub fn put() -> Self {
        Self::with_method(Method::PUT)
    }

    pub fn patch() -> Self {
        Self::with_method(Method::PATCH)
    }

    pub fn delete() -> Self {
        Self::with_method(Method::DELETE)
    }

    #[must_use]
    pub fn method(mut self, method: Method) -> Self {
        self.builder = self.builder.method(method);
        self
    }

    #[must_use]
    pub fn uri(mut self, uri: &str) -> Self {
        self.builder = self.builder.uri(uri);
        self
    }

    #[must_use]
    pub fn insert_header<K, V>(mut self, (name, value): (K, V)) -> Self
    where
        K: TryInto<HeaderName>,
        K::Error: Into<axum::http::Error>,
        V: TryInto<HeaderValue>,
        V::Error: Into<axum::http::Error>,
    {
        self.builder = self.builder.header(name, value);
        self
    }

    /// A urlencoded body, with its `content-type`.
    #[must_use]
    pub fn set_form<T: serde::Serialize>(mut self, form: T) -> Self {
        let body = serde_urlencoded::to_string(&form).expect("form serializes");
        self.builder = self
            .builder
            .header("content-type", "application/x-www-form-urlencoded");
        self.body = Body::from(body);
        self
    }

    #[must_use]
    pub fn set_payload(mut self, payload: impl Into<Bytes>) -> Self {
        self.body = Body::from(payload.into());
        self
    }

    // actix's name, so the ported tests read as they did.
    #[allow(clippy::wrong_self_convention)]
    pub fn to_request(self) -> Request<Body> {
        self.builder.body(self.body).expect("valid test request")
    }
}

/// The router itself: there is nothing to initialize under axum.
pub async fn init_service(app: Router) -> Router {
    app
}

/// Send `req` through `app` (a fresh clone of the router per call).
pub async fn call_service(app: &Router, req: Request<Body>) -> Response {
    app.clone()
        .oneshot(req)
        .await
        .expect("the router is infallible")
}

pub async fn read_body(response: Response) -> Bytes {
    response
        .into_body()
        .collect()
        .await
        .expect("the response body reads")
        .to_bytes()
}

pub async fn read_body_json<T: serde::de::DeserializeOwned>(response: Response) -> T {
    serde_json::from_slice(&read_body(response).await).expect("a JSON response body")
}
