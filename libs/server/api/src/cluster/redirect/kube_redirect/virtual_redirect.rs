//! Serving the virtual APIs declared on a cluster.
//!
//! Three shapes, all decided from the request path alone:
//!
//! - discovery the proxy owns (`/apis/{group}`, `/apis/{group}/{version}`) is
//!   answered without touching the cluster;
//! - `GET /apis` is fetched upstream and the virtual groups merged into it;
//! - a mapped resource request is rewritten, forwarded, and its response
//!   rewritten back — buffered for a normal request, event by event for a watch.
//!
//! Response rewriting means buffering, which is why watches get their own
//! newline-delimited path: a watch never ends, so it can never be buffered.

use super::RedirectContext;
use actix_web::{HttpRequest, HttpResponse, dev::PeerAddr, http, web};
use common::State;
use crd::ProxyKubeApi;
use futures_util::StreamExt;
use serde_json::Value;
use tracing::{debug, error, warn};
use virtual_api::MapperRegistry;
use virtual_api::discovery::{
    DiscoveryRequest, api_group_response, api_resource_list_response, classify,
    merge_api_group_list,
};

use super::upstream::{apply_forward_headers, upstream_client};
use crate::cluster::redirect::audit::AuditContext;
use crate::model::user::User;

/// Upper bound on a buffered virtual response (`PROXY_VIRTUAL_MAX_BODY_BYTES`,
/// default 32 MiB, see `common::config`).
///
/// A translated response must be held in memory in full; a `NamespaceList` on a
/// very large cluster is the realistic worst case, and past this the request is
/// refused rather than allowed to grow without bound.
fn max_buffered_bytes() -> usize {
    common::config::get().proxy.virtual_max_body_bytes
}

/// What the proxy will do with a request a virtual API claims.
pub(super) enum VirtualPlan {
    /// Answer from the mapper, no upstream call at all.
    Direct(Value),
    /// Fetch `/apis` upstream, then merge the virtual groups into it.
    MergeApiGroups,
    /// Rewrite the request onto `upstream_path` and translate the response.
    Mapped { upstream_path: String },
    /// The path is a known virtual resource but the method is not one it
    /// supports; answer `405` (with this `Allow` header) without any upstream call.
    MethodNotAllowed { allow: String },
}

impl VirtualPlan {
    /// The path actually reached on the target cluster, when there is one.
    ///
    /// Used to run the resource allow-list against what is really accessed, not
    /// only against the virtual path the client typed.
    pub(super) fn upstream_path(&self) -> Option<&str> {
        match self {
            VirtualPlan::Direct(_) => None,
            VirtualPlan::MergeApiGroups => Some("/apis"),
            VirtualPlan::Mapped { upstream_path } => Some(upstream_path),
            VirtualPlan::MethodNotAllowed { .. } => None,
        }
    }
}

/// Decide what a virtual API does with `path`, if anything.
///
/// `path` must be free of its query string.
pub(super) fn plan(registry: &MapperRegistry, path: &str, method: &str) -> Option<VirtualPlan> {
    if registry.is_empty() {
        return None;
    }

    if method.eq_ignore_ascii_case("GET") {
        match classify(registry, path) {
            Some(DiscoveryRequest::ApiGroupList) => return Some(VirtualPlan::MergeApiGroups),
            Some(DiscoveryRequest::ApiGroup(group)) => {
                let mapper = registry.find_group(&group)?;
                return Some(VirtualPlan::Direct(api_group_response(mapper)));
            }
            Some(DiscoveryRequest::ApiResourceList(group, version)) => {
                let mapper = registry.find_group_version(&group, &version)?;
                return Some(VirtualPlan::Direct(api_resource_list_response(mapper)));
            }
            None => {}
        }
    }

    let (mapper, mut route) = registry.resolve(path)?;
    route.method = method.to_ascii_uppercase();
    if let Some(allow) = mapper.method_not_allowed(&route) {
        return Some(VirtualPlan::MethodNotAllowed { allow });
    }
    Some(VirtualPlan::Mapped {
        upstream_path: mapper.map_request(&route).path,
    })
}

/// `true` when the request asks for a watch stream.
fn is_watch(query_string: &str) -> bool {
    query_string
        .split('&')
        .any(|param| matches!(param, "watch=true" | "watch=1"))
}

fn json_response(status: http::StatusCode, body: &Value) -> HttpResponse {
    HttpResponse::build(status)
        .content_type("application/json")
        .body(body.to_string())
}

/// Read the client body, capped, so a mapper can rewrite it.
async fn read_client_body(
    payload: &mut web::Payload,
    limit: usize,
) -> Result<web::Bytes, ReadCapError> {
    let mut body = web::BytesMut::new();
    while let Some(chunk) = payload.next().await {
        let chunk = chunk.map_err(|err| ReadCapError::Upstream(err.to_string()))?;
        if body.len() + chunk.len() > limit {
            return Err(ReadCapError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body.freeze())
}

/// Why a capped upstream read stopped short.
enum ReadCapError {
    /// The body exceeded `limit`; aborted without buffering the rest.
    TooLarge,
    /// The upstream connection failed mid-read.
    Upstream(String),
}

/// Read an upstream response into memory, aborting as soon as it exceeds `limit`
/// instead of buffering the whole body first. A translated response must be held
/// in full, so without an incremental cap a multi-GB `NamespaceList` (or a
/// hostile upstream) would be read entirely into RAM before the size was checked.
async fn read_response_capped(
    res: reqwest::Response,
    limit: usize,
) -> Result<web::Bytes, ReadCapError> {
    // Reject early when the upstream announced an oversized body.
    if let Some(len) = res.content_length()
        && len > limit as u64
    {
        return Err(ReadCapError::TooLarge);
    }
    let mut body = web::BytesMut::new();
    let mut stream = res.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|err| ReadCapError::Upstream(err.to_string()))?;
        if body.len() + chunk.len() > limit {
            return Err(ReadCapError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body.freeze())
}

/// Where a virtual request is forwarded, once the local answers are ruled out.
enum Forward {
    /// `GET /apis`, merged with the virtual groups.
    MergeApiGroups,
    /// A resource request rewritten onto `mapped_path`.
    Mapped { mapped_path: String },
}

impl Forward {
    /// The path requested on the cluster.
    fn path(&self) -> &str {
        match self {
            Forward::MergeApiGroups => "/apis",
            Forward::Mapped { mapped_path } => mapped_path,
        }
    }
}

/// Why a forwarded virtual request failed. [`VirtualFailure::respond`] owns the
/// log line, audit status and client response of each case.
#[derive(Debug)]
enum VirtualFailure {
    /// A mapped plan whose path no mapper resolves any more.
    NoMapper { path: String },
    /// The client body is over the buffering cap.
    RequestBodyTooLarge { limit: usize },
    /// The client body could not be read.
    RequestBodyUnreadable(String),
    /// The upstream client could not be built.
    ClientUnavailable(String),
    /// The method has no reqwest equivalent.
    UnsupportedMethod { error: String, method: String },
    /// The upstream request could not be built.
    BuildFailed(reqwest::Error),
    /// The upstream request failed.
    ForwardFailed(reqwest::Error),
    /// The upstream response is over the buffering cap.
    ResponseTooLarge { limit: usize },
    /// The upstream response could not be read.
    ResponseUnreadable(String),
}

impl VirtualFailure {
    /// Log the failure, record its audit event and build the client answer.
    fn respond(self, audit: &AuditContext) -> HttpResponse {
        match self {
            Self::NoMapper { path } => {
                error!(%path, "virtual plan without a matching mapper");
                audit.emit(500);
                HttpResponse::InternalServerError().finish()
            }
            Self::RequestBodyTooLarge { limit } => {
                warn!(limit, "virtual request body exceeds the size cap");
                audit.emit(413);
                HttpResponse::PayloadTooLarge().body(format!("request body exceeds {limit} bytes"))
            }
            Self::RequestBodyUnreadable(err) => {
                warn!(%err, "could not read the request body of a virtual request");
                audit.emit(400);
                HttpResponse::BadRequest().body("could not read request body")
            }
            Self::ClientUnavailable(err) => {
                error!(err, "couldn't build the upstream client");
                audit.emit(503);
                HttpResponse::ServiceUnavailable().body("upstream unavailable")
            }
            Self::UnsupportedMethod { error, method } => {
                error!(error = %error, method = %method, "unsupported HTTP method");
                audit.emit(405);
                HttpResponse::MethodNotAllowed().body("unsupported HTTP method")
            }
            Self::BuildFailed(err) => {
                error!(error = %err, "couldn't build the virtual API request");
                audit.emit(500);
                HttpResponse::InternalServerError().finish()
            }
            Self::ForwardFailed(err) => {
                error!(error = %err, "error forwarding a virtual API request");
                audit.emit(503);
                HttpResponse::ServiceUnavailable().body("upstream unavailable")
            }
            Self::ResponseTooLarge { limit } => {
                error!(limit, "virtual API response is too large to translate");
                audit.emit(502);
                HttpResponse::BadGateway().body("upstream response is too large to translate")
            }
            Self::ResponseUnreadable(err) => {
                error!(error = %err, "error reading the upstream response");
                audit.emit(503);
                HttpResponse::ServiceUnavailable().body("upstream unavailable")
            }
        }
    }
}

/// Discovery the proxy owns outright: no cluster round-trip.
fn answer_discovery(body: &Value, upstream_path: &str, audit: &AuditContext) -> HttpResponse {
    debug!(path = %upstream_path, "answering virtual discovery locally");
    audit.emit(200);
    json_response(http::StatusCode::OK, body)
}

/// A verb the virtual resource does not support: refused before forwarding, so
/// it cannot be mapped onto an unrelated upstream operation.
fn method_not_allowed(
    allow: String,
    upstream_path: &str,
    method: &http::Method,
    audit: &AuditContext,
) -> HttpResponse {
    debug!(path = %upstream_path, method = %method.as_str(), "method not allowed on virtual resource");
    audit.emit(405);
    HttpResponse::MethodNotAllowed()
        .insert_header(("allow", allow))
        .finish()
}

/// What to forward for a client body: nothing when empty, the mapper's
/// rewrite when it is JSON, the bytes verbatim otherwise (the cluster will say
/// what it thinks of them).
fn rewrite_request_body(
    body: web::Bytes,
    rewrite: impl FnOnce(Value) -> Value,
) -> Option<VirtualRequestBody> {
    if body.is_empty() {
        return None;
    }
    match serde_json::from_slice::<Value>(&body) {
        Ok(json) => Some(VirtualRequestBody::Rewritten(
            rewrite(json).to_string().into_bytes(),
        )),
        Err(_) => Some(VirtualRequestBody::Verbatim(body.to_vec())),
    }
}

/// Read the client body of a mapped request, capped, and let the mapper
/// rewrite it (a ProjectRequest becoming a Namespace). `GET /apis` forwards none.
async fn forwarded_body(
    forward: &Forward,
    registry: &MapperRegistry,
    upstream_path: &str,
    method: &http::Method,
    payload: &mut web::Payload,
) -> Result<Option<VirtualRequestBody>, VirtualFailure> {
    if matches!(forward, Forward::MergeApiGroups) {
        return Ok(None);
    }
    let limit = max_buffered_bytes();
    let Some((mapper, mut route)) = registry.resolve(upstream_path) else {
        return Err(VirtualFailure::NoMapper {
            path: upstream_path.to_string(),
        });
    };
    route.method = method.as_str().to_ascii_uppercase();

    let body = read_client_body(payload, limit)
        .await
        .map_err(|err| match err {
            ReadCapError::TooLarge => VirtualFailure::RequestBodyTooLarge { limit },
            ReadCapError::Upstream(err) => VirtualFailure::RequestBodyUnreadable(err),
        })?;
    Ok(rewrite_request_body(body, |json| {
        mapper.map_request_body(&route, json)
    }))
}

/// `{base_url}{path}`, with the client's query string when there is one.
fn upstream_url(base_url: &str, path: &str, query_string: &str) -> String {
    if query_string.is_empty() {
        format!("{base_url}{path}")
    } else {
        format!("{base_url}{path}?{query_string}")
    }
}

/// The client request a virtual call is forwarded on behalf of.
struct Caller<'a> {
    req: &'a HttpRequest,
    method: &'a http::Method,
    peer_addr: Option<PeerAddr>,
    user: Option<&'a User>,
}

/// Build the upstream request, send it, and hand back the upstream answer.
async fn send_upstream(
    proxy: &ProxyKubeApi,
    data: &web::Data<State>,
    url: &str,
    caller: &Caller<'_>,
    request_body: Option<VirtualRequestBody>,
) -> Result<reqwest::Response, VirtualFailure> {
    let client = upstream_client(proxy, data)
        .await
        .map_err(VirtualFailure::ClientUnavailable)?;
    let forwarded_req = build_forwarded_request(&client, url, caller, request_body)?;

    debug!(from = %caller.req.path(), to = %url, "forwarding a virtual API request");
    client
        .execute(forwarded_req)
        .await
        .map_err(VirtualFailure::ForwardFailed)
}

/// Build the upstream request: forwarded headers, identity encoding, body.
fn build_forwarded_request(
    client: &reqwest::Client,
    url: &str,
    caller: &Caller<'_>,
    request_body: Option<VirtualRequestBody>,
) -> Result<reqwest::Request, VirtualFailure> {
    let method = caller.method;
    let upstream_method =
        reqwest::Method::from_bytes(method.as_str().as_bytes()).map_err(|err| {
            VirtualFailure::UnsupportedMethod {
                error: err.to_string(),
                method: method.as_str().to_string(),
            }
        })?;
    let mut forwarded_req = apply_forward_headers(
        client.request(upstream_method, url),
        caller.req,
        caller.peer_addr,
        caller.user,
    )
    .build()
    .map_err(VirtualFailure::BuildFailed)?;
    // The response is parsed and rewritten here, and reqwest is built without
    // decompression: a compressed upstream answer could neither be translated
    // nor be forwarded as-is (its `Content-Encoding` is not passed back), so
    // ask for an uncompressed one whatever the client accepts.
    forwarded_req.headers_mut().insert(
        reqwest::header::ACCEPT_ENCODING,
        reqwest::header::HeaderValue::from_static("identity"),
    );
    attach_body(&mut forwarded_req, request_body);
    Ok(forwarded_req)
}

fn attach_body(forwarded_req: &mut reqwest::Request, request_body: Option<VirtualRequestBody>) {
    match request_body {
        Some(VirtualRequestBody::Rewritten(body)) => {
            // `insert` replaces the client's own `Content-Type`; the mapper
            // always produces JSON.
            forwarded_req.headers_mut().insert(
                reqwest::header::CONTENT_TYPE,
                reqwest::header::HeaderValue::from_static("application/json"),
            );
            *forwarded_req.body_mut() = Some(body.into());
        }
        Some(VirtualRequestBody::Verbatim(body)) => {
            *forwarded_req.body_mut() = Some(body.into());
        }
        None => {}
    }
}

/// Rewrite a JSON upstream answer back into the virtual API's shape.
fn translate_json(
    forward: &Forward,
    registry: &MapperRegistry,
    upstream_path: &str,
    json: Value,
) -> Value {
    match forward {
        Forward::MergeApiGroups => merge_api_group_list(registry, json),
        Forward::Mapped { .. } => match registry.resolve(upstream_path) {
            Some((mapper, _)) => mapper.map_response(json),
            None => json,
        },
    }
}

/// Buffer the upstream answer (capped) and translate it.
async fn translated_response(
    res: reqwest::Response,
    status: http::StatusCode,
    forward: &Forward,
    registry: &MapperRegistry,
    upstream_path: &str,
    audit: &AuditContext,
) -> Result<HttpResponse, VirtualFailure> {
    let limit = max_buffered_bytes();
    let body = read_response_capped(res, limit)
        .await
        .map_err(|err| match err {
            ReadCapError::TooLarge => VirtualFailure::ResponseTooLarge { limit },
            ReadCapError::Upstream(err) => VirtualFailure::ResponseUnreadable(err),
        })?;

    let json: Value = match serde_json::from_slice(&body) {
        Ok(json) => json,
        Err(err) => {
            // Nothing to translate; hand the bytes back as they came.
            warn!(error = %err, "upstream response is not JSON, forwarding it untouched");
            audit.emit(status.as_u16());
            return Ok(HttpResponse::build(status).body(body));
        }
    };

    let translated = translate_json(forward, registry, upstream_path, json);
    audit.emit(status.as_u16());
    Ok(json_response(status, &translated))
}

pub(super) async fn virtual_redirect(
    ctx: RedirectContext,
    base_url: String,
    registry: MapperRegistry,
    upstream_path: String,
    plan: VirtualPlan,
) -> HttpResponse {
    let RedirectContext {
        req,
        data,
        mut payload,
        method,
        peer_addr,
        proxy,
        user,
        audit,
    } = ctx;
    let forward = match plan {
        VirtualPlan::Direct(body) => return answer_discovery(&body, &upstream_path, &audit),
        VirtualPlan::MethodNotAllowed { allow } => {
            return method_not_allowed(allow, &upstream_path, &method, &audit);
        }
        VirtualPlan::MergeApiGroups => Forward::MergeApiGroups,
        VirtualPlan::Mapped {
            upstream_path: mapped_path,
        } => Forward::Mapped { mapped_path },
    };

    let outcome = async {
        let request_body =
            forwarded_body(&forward, &registry, &upstream_path, &method, &mut payload).await?;
        let query_string = req.query_string();
        let url = upstream_url(&base_url, forward.path(), query_string);
        let caller = Caller {
            req: &req,
            method: &method,
            peer_addr,
            user: user.as_ref(),
        };
        let res = send_upstream(&proxy, &data, &url, &caller, request_body).await?;
        let status = http::StatusCode::from_u16(res.status().as_u16())
            .unwrap_or(http::StatusCode::BAD_GATEWAY);

        // Each branch emits the audit event for the status it really answers
        // with, exactly once. A watch never ends, so it is translated event by
        // event as it flows.
        if matches!(forward, Forward::Mapped { .. }) && is_watch(query_string) {
            audit.emit(status.as_u16());
            return Ok(stream_watch(res, status, registry, upstream_path));
        }
        translated_response(res, status, &forward, &registry, &upstream_path, &audit).await
    };

    outcome
        .await
        .unwrap_or_else(|failure: VirtualFailure| failure.respond(&audit))
}

/// The body forwarded with a virtual API request.
#[derive(Debug, PartialEq)]
enum VirtualRequestBody {
    /// Rewritten by the mapper; always JSON.
    Rewritten(Vec<u8>),
    /// Not JSON, so passed through untouched with the client's `Content-Type`.
    Verbatim(Vec<u8>),
}

/// Translate a newline-delimited watch stream event by event.
fn stream_watch(
    res: reqwest::Response,
    status: http::StatusCode,
    registry: MapperRegistry,
    upstream_path: String,
) -> HttpResponse {
    let mut buffer = web::BytesMut::new();
    // Cap a single unterminated line: a watch that never emits a newline (a
    // hostile or stuck upstream) would otherwise grow `buffer` without bound.
    let line_limit = max_buffered_bytes();
    let translated = res.bytes_stream().map(move |chunk| {
        let chunk = chunk.map_err(actix_web::error::ErrorBadGateway)?;
        buffer.extend_from_slice(&chunk);

        let mut out = web::BytesMut::new();
        // Only whole lines can be parsed; a partial one stays buffered until
        // the rest of it arrives.
        while let Some(newline) = buffer.iter().position(|byte| *byte == b'\n') {
            let line = buffer.split_to(newline + 1);
            let trimmed = &line[..line.len() - 1];
            if trimmed.is_empty() {
                continue;
            }

            match serde_json::from_slice::<Value>(trimmed) {
                Ok(event) => {
                    let event = match registry.resolve(&upstream_path) {
                        Some((mapper, _)) => mapper.map_watch_event(event),
                        None => event,
                    };
                    out.extend_from_slice(event.to_string().as_bytes());
                    out.extend_from_slice(b"\n");
                }
                Err(err) => {
                    // Pass unparseable lines through untouched rather than
                    // silently dropping part of the stream.
                    warn!(error = %err, "watch event is not JSON, forwarding it untouched");
                    out.extend_from_slice(&line);
                }
            }
        }

        // Whatever is left is an as-yet-unterminated line; refuse to let it grow
        // past the limit.
        if buffer.len() > line_limit {
            return Err(actix_web::error::ErrorBadGateway(
                "watch line exceeds the size limit",
            ));
        }

        Ok::<web::Bytes, actix_web::Error>(out.freeze())
    });

    HttpResponse::build(status)
        .content_type("application/json")
        // Stop actix' Compress middleware from buffering the stream.
        .insert_header(("content-encoding", "identity"))
        .streaming(translated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crd::virtual_api::VirtualApiKind;

    fn registry() -> MapperRegistry {
        MapperRegistry::from_kinds(&[VirtualApiKind::OpenShiftProject])
    }

    #[test]
    fn an_empty_registry_plans_nothing() {
        assert!(plan(&MapperRegistry::new(), "/apis", "GET").is_none());
    }

    #[test]
    fn an_unsupported_verb_plans_a_405() {
        let plan = plan(
            &registry(),
            "/apis/project.openshift.io/v1/projectrequests",
            "DELETE",
        )
        .expect("a known virtual path should still be planned");
        match &plan {
            VirtualPlan::MethodNotAllowed { allow } => assert_eq!(allow, "GET, POST"),
            _ => panic!("DELETE on projectrequests must be refused"),
        }
        assert_eq!(plan.upstream_path(), None);
    }

    #[test]
    fn discovery_of_a_served_group_is_answered_locally() {
        let plan = plan(&registry(), "/apis/project.openshift.io", "GET")
            .expect("group discovery should be planned");
        assert!(matches!(plan, VirtualPlan::Direct(_)));
        assert_eq!(plan.upstream_path(), None);
    }

    #[test]
    fn the_group_list_is_merged_with_the_cluster() {
        let plan = plan(&registry(), "/apis", "GET").expect("group list should be planned");
        assert!(matches!(plan, VirtualPlan::MergeApiGroups));
        assert_eq!(plan.upstream_path(), Some("/apis"));
    }

    #[test]
    fn a_resource_request_is_mapped_onto_the_real_api() {
        let plan = plan(
            &registry(),
            "/apis/project.openshift.io/v1/projects/dev",
            "GET",
        )
        .expect("project should be planned");
        assert_eq!(plan.upstream_path(), Some("/api/v1/namespaces/dev"));
    }

    #[test]
    fn a_project_request_maps_onto_a_namespace_creation() {
        let plan = plan(
            &registry(),
            "/apis/project.openshift.io/v1/projectrequests",
            "POST",
        )
        .expect("project request should be planned");
        assert_eq!(plan.upstream_path(), Some("/api/v1/namespaces"));
    }

    #[test]
    fn real_api_paths_are_left_to_the_standard_proxy() {
        let registry = registry();
        assert!(plan(&registry, "/api/v1/namespaces", "GET").is_none());
        assert!(plan(&registry, "/apis/apps/v1/deployments", "GET").is_none());
        // Discovery only applies to GET.
        assert!(plan(&registry, "/apis/project.openshift.io", "POST").is_none());
    }

    fn audit() -> AuditContext {
        AuditContext::new("ns", "c", "GET", "/apis")
    }

    #[test]
    fn the_upstream_url_keeps_the_query_only_when_present() {
        assert_eq!(
            upstream_url("https://k:6443", "/api/v1/namespaces", ""),
            "https://k:6443/api/v1/namespaces"
        );
        assert_eq!(
            upstream_url("https://k:6443", "/apis", "limit=5"),
            "https://k:6443/apis?limit=5"
        );
    }

    #[test]
    fn a_forward_targets_its_upstream_path() {
        assert_eq!(Forward::MergeApiGroups.path(), "/apis");
        let mapped = Forward::Mapped {
            mapped_path: "/api/v1/namespaces".to_string(),
        };
        assert_eq!(mapped.path(), "/api/v1/namespaces");
    }

    #[test]
    fn client_bodies_are_rewritten_only_when_json() {
        let rewrite = |_: Value| serde_json::json!({"kind": "Namespace"});
        assert_eq!(rewrite_request_body(web::Bytes::new(), rewrite), None);
        assert_eq!(
            rewrite_request_body(web::Bytes::from_static(b"{}"), rewrite),
            Some(VirtualRequestBody::Rewritten(
                br#"{"kind":"Namespace"}"#.to_vec()
            ))
        );
        assert_eq!(
            rewrite_request_body(web::Bytes::from_static(b"not json"), rewrite),
            Some(VirtualRequestBody::Verbatim(b"not json".to_vec()))
        );
    }

    #[test]
    fn a_rewritten_body_is_sent_as_json_and_a_verbatim_one_keeps_its_type() {
        let client = reqwest::Client::new();
        let mut req = client
            .post("http://k/x")
            .header("content-type", "text/plain")
            .build()
            .unwrap();
        attach_body(
            &mut req,
            Some(VirtualRequestBody::Verbatim(b"raw".to_vec())),
        );
        assert_eq!(req.headers()["content-type"], "text/plain");
        assert_eq!(req.body().and_then(|b| b.as_bytes()), Some(&b"raw"[..]));

        attach_body(
            &mut req,
            Some(VirtualRequestBody::Rewritten(b"{}".to_vec())),
        );
        assert_eq!(req.headers()["content-type"], "application/json");
        assert_eq!(req.body().and_then(|b| b.as_bytes()), Some(&b"{}"[..]));
    }

    #[test]
    fn an_unresolved_mapped_response_is_left_untouched() {
        let json = serde_json::json!({"kind": "Namespace"});
        let forward = Forward::Mapped {
            mapped_path: "/api/v1/namespaces".to_string(),
        };
        let out = translate_json(&forward, &registry(), "/api/v1/namespaces", json.clone());
        assert_eq!(out, json);
    }

    #[test]
    fn local_answers_keep_their_status_and_headers() {
        let audit = audit();
        let answer = answer_discovery(&serde_json::json!({}), "/apis/x", &audit);
        assert_eq!(answer.status(), http::StatusCode::OK);
        assert_eq!(
            answer.headers().get("content-type").unwrap(),
            "application/json"
        );

        let refused =
            method_not_allowed("GET, POST".into(), "/apis/x", &http::Method::DELETE, &audit);
        assert_eq!(refused.status(), http::StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(refused.headers().get("allow").unwrap(), "GET, POST");
    }

    #[test]
    fn each_failure_keeps_its_status() {
        let audit = audit();
        let cases = [
            (VirtualFailure::NoMapper { path: "/p".into() }, 500),
            (VirtualFailure::RequestBodyTooLarge { limit: 1 }, 413),
            (VirtualFailure::RequestBodyUnreadable("e".into()), 400),
            (VirtualFailure::ClientUnavailable("e".into()), 503),
            (
                VirtualFailure::UnsupportedMethod {
                    error: "e".into(),
                    method: "X".into(),
                },
                405,
            ),
            (VirtualFailure::ResponseTooLarge { limit: 1 }, 502),
            (VirtualFailure::ResponseUnreadable("e".into()), 503),
        ];
        for (failure, status) in cases {
            let label = format!("{failure:?}");
            assert_eq!(failure.respond(&audit).status().as_u16(), status, "{label}");
        }
    }

    #[test]
    fn watch_detection_only_accepts_the_real_parameter() {
        assert!(is_watch("watch=true"));
        assert!(is_watch("resourceVersion=1&watch=1"));
        assert!(!is_watch("watch=false"));
        assert!(!is_watch("allowWatchBookmarks=true"));
        assert!(!is_watch(""));
    }
}
