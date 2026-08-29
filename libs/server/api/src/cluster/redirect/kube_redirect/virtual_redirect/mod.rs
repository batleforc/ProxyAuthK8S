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

use actix_web::{HttpResponse, http};
use serde_json::Value;
use tracing::{debug, error, warn};
use virtual_api::MapperRegistry;
use virtual_api::discovery::merge_api_group_list;

use super::context::RedirectContext;
use super::list_fallback;
use super::upstream::{apply_forward_headers, upstream_client};

mod body;
mod plan;
mod watch;

use body::{
    ReadCapError, json_response, max_buffered_bytes, read_client_body, read_response_capped,
};
use plan::is_watch;
pub(super) use plan::{VirtualPlan, plan};
use watch::stream_watch;

pub(super) async fn virtual_redirect(
    ctx: RedirectContext,
    registry: MapperRegistry,
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
        base_url,
        upstream_path,
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

    // Resolved once and reused for both the request-body rewrite below and
    // the list-fallback check further down, rather than resolving the same
    // `upstream_path` against the registry twice per request.
    let resolved = match &plan {
        VirtualPlan::Mapped { .. } => {
            let Some((mapper, mut route)) = registry.resolve(&upstream_path) else {
                error!(path = %upstream_path, "virtual plan without a matching mapper");
                audit.emit(500);
                return HttpResponse::InternalServerError().finish();
            };
            route.method = method.as_str().to_ascii_uppercase();
            Some((mapper, route))
        }
        _ => None,
    };

    // A mapper may need to rewrite the client body (a ProjectRequest becoming a
    // Namespace); the body has to be read in full for that.
    let (mapped_path, request_body) = match &plan {
        VirtualPlan::MergeApiGroups => ("/apis".to_string(), None),
        VirtualPlan::Mapped {
            upstream_path: mapped,
        } => {
            let (mapper, route) = resolved
                .as_ref()
                .expect("resolved is Some for a Mapped plan, set above");

            let body = match read_client_body(&mut payload, limit).await {
                Ok(body) if body.is_empty() => None,
                Ok(body) => match serde_json::from_slice::<Value>(&body) {
                    Ok(json) => Some(
                        mapper
                            .map_request_body(route, json)
                            .to_string()
                            .into_bytes(),
                    ),
                    // Not JSON: forward verbatim rather than reject, the cluster
                    // will say what it thinks of it.
                    Err(_) => Some(body.to_vec()),
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

    // A mapper offering per-item visibility filtering, on a cluster that has
    // opted into it: skip the plain forward entirely and always return the
    // filtered collection, whether or not the caller could also have listed
    // it directly. See `list_fallback` for why this can't just be a 403
    // rescue.
    if let Some((mapper, route)) = resolved.as_ref()
        && method.as_str().eq_ignore_ascii_case("GET")
        && !is_watch(query_string)
        && list_fallback::is_configured(&proxy)
        && let Some(probe) = mapper.list_access_probe(route)
    {
        // The same path mapping the plain forward would use for this route —
        // derived here instead of duplicated as a literal, so it can never
        // drift from `OpenShiftProjectMapper`'s own mapping.
        let namespaces_path = mapper.map_request(route).path;
        return match list_fallback::list_projects_filtered(
            &proxy,
            &data,
            &client,
            &req,
            peer_addr,
            user.as_ref(),
            &base_url,
            &namespaces_path,
            query_string,
            probe,
        )
        .await
        {
            Ok(json) => {
                audit.emit(200);
                json_response(http::StatusCode::OK, &mapper.map_response(json))
            }
            Err(list_fallback::DiscoveryError(err)) => {
                error!(err, "list fallback discovery failed");
                audit.emit(503);
                HttpResponse::ServiceUnavailable().body("upstream unavailable")
            }
        };
    }
    // Else: this route has no per-item probe (e.g. a single-object GET), or
    // the mapper isn't `Mapped`, or the request isn't a plain GET; fall
    // through to the plain forward below, unchanged.

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
            return HttpResponse::ServiceUnavailable().body("upstream unavailable");
        }
    };

    let status =
        http::StatusCode::from_u16(res.status().as_u16()).unwrap_or(http::StatusCode::BAD_GATEWAY);
    audit.emit(status.as_u16());

    // A watch never ends, so it is translated event by event as it flows.
    if matches!(plan, VirtualPlan::Mapped { .. }) && is_watch(query_string) {
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
            return HttpResponse::ServiceUnavailable().body("upstream unavailable");
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
        // Handled by the early returns above; guard defensively rather than panic.
        VirtualPlan::Direct(_) | VirtualPlan::MethodNotAllowed { .. } => {
            error!(path = %upstream_path, "virtual plan reached response translation unexpectedly");
            audit.emit(500);
            return HttpResponse::InternalServerError().finish();
        }
    };

    json_response(status, &translated)
}
