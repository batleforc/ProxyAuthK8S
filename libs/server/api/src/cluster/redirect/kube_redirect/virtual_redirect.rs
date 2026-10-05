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
use actix_web::{HttpResponse, http, web};
use futures_util::StreamExt;
use serde_json::Value;
use tracing::{debug, error, warn};
use virtual_api::MapperRegistry;
use virtual_api::discovery::{
    DiscoveryRequest, api_group_response, api_resource_list_response, classify,
    merge_api_group_list,
};

use super::upstream::{apply_forward_headers, upstream_client};

/// Upper bound on a buffered virtual response.
///
/// A translated response must be held in memory in full; a `NamespaceList` on a
/// very large cluster is the realistic worst case, and past this the request is
/// refused rather than allowed to grow without bound.
const DEFAULT_MAX_BUFFERED_BYTES: usize = 32 * 1024 * 1024;

fn max_buffered_bytes() -> usize {
    std::env::var("PROXY_VIRTUAL_MAX_BODY_BYTES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_MAX_BUFFERED_BYTES)
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
    // Discovery the proxy owns outright: no cluster round-trip.
    if let VirtualPlan::Direct(body) = &plan {
        debug!(path = %upstream_path, "answering virtual discovery locally");
        audit.emit(200);
        return json_response(http::StatusCode::OK, body);
    }

    // A verb the virtual resource does not support: refuse before forwarding, so
    // it cannot be mapped onto an unrelated upstream operation.
    if let VirtualPlan::MethodNotAllowed { allow } = &plan {
        debug!(path = %upstream_path, method = %method.as_str(), "method not allowed on virtual resource");
        audit.emit(405);
        return HttpResponse::MethodNotAllowed()
            .insert_header(("allow", allow.clone()))
            .finish();
    }

    let limit = max_buffered_bytes();

    // A mapper may need to rewrite the client body (a ProjectRequest becoming a
    // Namespace); the body has to be read in full for that.
    let (mapped_path, request_body) = match &plan {
        VirtualPlan::MergeApiGroups => ("/apis".to_string(), None),
        VirtualPlan::Mapped {
            upstream_path: mapped,
        } => {
            let Some((mapper, mut route)) = registry.resolve(&upstream_path) else {
                error!(path = %upstream_path, "virtual plan without a matching mapper");
                audit.emit(500);
                return HttpResponse::InternalServerError().finish();
            };
            route.method = method.as_str().to_ascii_uppercase();

            let body = match read_client_body(&mut payload, limit).await {
                Ok(body) if body.is_empty() => None,
                Ok(body) => match serde_json::from_slice::<Value>(&body) {
                    Ok(json) => Some(VirtualRequestBody::Rewritten(
                        mapper
                            .map_request_body(&route, json)
                            .to_string()
                            .into_bytes(),
                    )),
                    // Not JSON: forward verbatim rather than reject, the cluster
                    // will say what it thinks of it.
                    Err(_) => Some(VirtualRequestBody::Verbatim(body.to_vec())),
                },
                Err(ReadCapError::TooLarge) => {
                    warn!(limit, "virtual request body exceeds the size cap");
                    audit.emit(413);
                    return HttpResponse::PayloadTooLarge()
                        .body(format!("request body exceeds {limit} bytes"));
                }
                Err(ReadCapError::Upstream(err)) => {
                    warn!(%err, "could not read the request body of a virtual request");
                    audit.emit(400);
                    return HttpResponse::BadRequest().body("could not read request body");
                }
            };

            (mapped.clone(), body)
        }
        // Handled by the early returns above; treated as a bug-guard rather than
        // a panic so a future refactor cannot crash a worker on the data path.
        VirtualPlan::Direct(_) | VirtualPlan::MethodNotAllowed { .. } => {
            error!(path = %upstream_path, "virtual plan reached the mapping stage unexpectedly");
            audit.emit(500);
            return HttpResponse::InternalServerError().finish();
        }
    };

    let query_string = req.query_string();
    let url = if query_string.is_empty() {
        format!("{base_url}{mapped_path}")
    } else {
        format!("{base_url}{mapped_path}?{query_string}")
    };

    let client = match upstream_client(&proxy, &data).await {
        Ok(client) => client,
        Err(err) => {
            error!(err, "couldn't build the upstream client");
            audit.emit(503);
            return HttpResponse::ServiceUnavailable().body("upstream unavailable");
        }
    };

    let upstream_method = match reqwest::Method::from_bytes(method.as_str().as_bytes()) {
        Ok(method) => method,
        Err(err) => {
            error!(error = %err, method = %method.as_str(), "unsupported HTTP method");
            audit.emit(405);
            return HttpResponse::MethodNotAllowed().body("unsupported HTTP method");
        }
    };

    let forwarded_req = apply_forward_headers(
        client.request(upstream_method, &url),
        &req,
        peer_addr,
        user.as_ref(),
    );
    let mut forwarded_req = match forwarded_req.build() {
        Ok(forwarded_req) => forwarded_req,
        Err(err) => {
            error!(error = %err, "couldn't build the virtual API request");
            audit.emit(500);
            return HttpResponse::InternalServerError().finish();
        }
    };
    // The response is parsed and rewritten here, and reqwest is built without
    // decompression: a compressed upstream answer could neither be translated
    // nor be forwarded as-is (its `Content-Encoding` is not passed back), so
    // ask for an uncompressed one whatever the client accepts.
    forwarded_req.headers_mut().insert(
        reqwest::header::ACCEPT_ENCODING,
        reqwest::header::HeaderValue::from_static("identity"),
    );
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

    debug!(from = %req.path(), to = %url, "forwarding a virtual API request");

    let res = match client.execute(forwarded_req).await {
        Ok(res) => res,
        Err(err) => {
            error!(error = %err, "error forwarding a virtual API request");
            audit.emit(503);
            return HttpResponse::ServiceUnavailable().body("upstream unavailable");
        }
    };

    let status =
        http::StatusCode::from_u16(res.status().as_u16()).unwrap_or(http::StatusCode::BAD_GATEWAY);

    // Each branch below emits the audit event for the status it really answers
    // with, exactly once.
    // A watch never ends, so it is translated event by event as it flows.
    if matches!(plan, VirtualPlan::Mapped { .. }) && is_watch(query_string) {
        audit.emit(status.as_u16());
        return stream_watch(res, status, registry, upstream_path);
    }

    let body = match read_response_capped(res, limit).await {
        Ok(body) => body,
        Err(ReadCapError::TooLarge) => {
            error!(limit, "virtual API response is too large to translate");
            audit.emit(502);
            return HttpResponse::BadGateway().body("upstream response is too large to translate");
        }
        Err(ReadCapError::Upstream(err)) => {
            error!(error = %err, "error reading the upstream response");
            audit.emit(503);
            return HttpResponse::ServiceUnavailable().body("upstream unavailable");
        }
    };

    let json: Value = match serde_json::from_slice(&body) {
        Ok(json) => json,
        Err(err) => {
            // Nothing to translate; hand the bytes back as they came.
            warn!(error = %err, "upstream response is not JSON, forwarding it untouched");
            audit.emit(status.as_u16());
            return HttpResponse::build(status).body(body);
        }
    };

    let translated = match &plan {
        VirtualPlan::MergeApiGroups => merge_api_group_list(&registry, json),
        VirtualPlan::Mapped { .. } => match registry.resolve(&upstream_path) {
            Some((mapper, _)) => mapper.map_response(json),
            None => json,
        },
        // Handled by the early returns above; guard defensively rather than panic.
        VirtualPlan::Direct(_) | VirtualPlan::MethodNotAllowed { .. } => {
            error!(path = %upstream_path, "virtual plan reached response translation unexpectedly");
            audit.emit(500);
            return HttpResponse::InternalServerError().finish();
        }
    };

    audit.emit(status.as_u16());
    json_response(status, &translated)
}

/// The body forwarded with a virtual API request.
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

    #[test]
    fn watch_detection_only_accepts_the_real_parameter() {
        assert!(is_watch("watch=true"));
        assert!(is_watch("resourceVersion=1&watch=1"));
        assert!(!is_watch("watch=false"));
        assert!(!is_watch("allowWatchBookmarks=true"));
        assert!(!is_watch(""));
    }
}
