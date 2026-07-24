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

use actix_web::{dev::PeerAddr, http, web, HttpRequest, HttpResponse};
use common::State;
use crd::ProxyKubeApi;
use futures_util::StreamExt;
use serde_json::Value;
use tracing::{debug, error, warn};
use virtual_api::discovery::{
    api_group_response, api_resource_list_response, classify, merge_api_group_list,
    DiscoveryRequest,
};
use virtual_api::MapperRegistry;

use super::upstream::{apply_forward_headers, upstream_client};
use crate::cluster::redirect::audit::AuditContext;
use crate::model::user::User;

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
async fn read_client_body(payload: &mut web::Payload, limit: usize) -> Result<web::Bytes, String> {
    let mut body = web::BytesMut::new();
    while let Some(chunk) = payload.next().await {
        let chunk = chunk.map_err(|err| err.to_string())?;
        if body.len() + chunk.len() > limit {
            return Err(format!("request body exceeds {limit} bytes"));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body.freeze())
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn virtual_redirect(
    req: HttpRequest,
    data: web::Data<State>,
    mut payload: web::Payload,
    method: http::Method,
    peer_addr: Option<PeerAddr>,
    proxy: ProxyKubeApi,
    base_url: String,
    user: Option<User>,
    audit: AuditContext,
    registry: MapperRegistry,
    upstream_path: String,
    plan: VirtualPlan,
) -> HttpResponse {
    // Discovery the proxy owns outright: no cluster round-trip.
    if let VirtualPlan::Direct(body) = &plan {
        debug!(path = %upstream_path, "answering virtual discovery locally");
        audit.emit(200);
        return json_response(http::StatusCode::OK, body);
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
                    Ok(json) => Some(
                        mapper
                            .map_request_body(&route, json)
                            .to_string()
                            .into_bytes(),
                    ),
                    // Not JSON: forward verbatim rather than reject, the cluster
                    // will say what it thinks of it.
                    Err(_) => Some(body.to_vec()),
                },
                Err(err) => {
                    warn!(%err, "could not read the request body of a virtual request");
                    audit.emit(413);
                    return HttpResponse::PayloadTooLarge().body(err);
                }
            };

            (mapped.clone(), body)
        }
        VirtualPlan::Direct(_) => unreachable!("handled above"),
    };

    let query_string = req.query_string();
    let url = if query_string.is_empty() {
        format!("{}{}", base_url, mapped_path)
    } else {
        format!("{}{}?{}", base_url, mapped_path, query_string)
    };

    let client = match upstream_client(&proxy, &data).await {
        Ok(client) => client,
        Err(err) => {
            error!(err, " couldn't build the upstream client");
            audit.emit(503);
            return HttpResponse::ServiceUnavailable().body(err);
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

    let mut forwarded_req = client.request(upstream_method, &url);
    forwarded_req = apply_forward_headers(forwarded_req, &req, peer_addr, user.as_ref());
    if let Some(body) = request_body {
        forwarded_req = forwarded_req
            .header(http::header::CONTENT_TYPE.as_str(), "application/json")
            .body(body);
    }

    debug!(from = %req.path(), to = %url, "forwarding a virtual API request");

    let res = match forwarded_req.send().await {
        Ok(res) => res,
        Err(err) => {
            error!(error = %err, "error forwarding a virtual API request");
            audit.emit(503);
            return HttpResponse::ServiceUnavailable().body(err.to_string());
        }
    };

    let status =
        http::StatusCode::from_u16(res.status().as_u16()).unwrap_or(http::StatusCode::BAD_GATEWAY);
    audit.emit(status.as_u16());

    // A watch never ends, so it is translated event by event as it flows.
    if matches!(plan, VirtualPlan::Mapped { .. }) && is_watch(query_string) {
        return stream_watch(res, status, registry, upstream_path);
    }

    let body = match res.bytes().await {
        Ok(body) if body.len() <= limit => body,
        Ok(body) => {
            error!(
                len = body.len(),
                limit, "virtual API response is too large to translate"
            );
            audit.emit(502);
            return HttpResponse::BadGateway().body("upstream response is too large to translate");
        }
        Err(err) => {
            error!(error = %err, "error reading the upstream response");
            return HttpResponse::ServiceUnavailable().body(err.to_string());
        }
    };

    let json: Value = match serde_json::from_slice(&body) {
        Ok(json) => json,
        Err(err) => {
            // Nothing to translate; hand the bytes back as they came.
            warn!(error = %err, "upstream response is not JSON, forwarding it untouched");
            return HttpResponse::build(status).body(body);
        }
    };

    let translated = match &plan {
        VirtualPlan::MergeApiGroups => merge_api_group_list(&registry, json),
        VirtualPlan::Mapped { .. } => match registry.resolve(&upstream_path) {
            Some((mapper, _)) => mapper.map_response(json),
            None => json,
        },
        VirtualPlan::Direct(_) => unreachable!("handled above"),
    };

    json_response(status, &translated)
}

/// Translate a newline-delimited watch stream event by event.
fn stream_watch(
    res: reqwest::Response,
    status: http::StatusCode,
    registry: MapperRegistry,
    upstream_path: String,
) -> HttpResponse {
    let mut buffer = web::BytesMut::new();
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
