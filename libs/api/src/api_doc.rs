use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(
      title = "ProxyAuthK8S",
      description = "Reverse Proxy K8s Api",
      contact(
            name = "Batleforc",
            email = "maxleriche.60@gmail.com",
            url = "https://maxleriche.net"
        )),
    tags(
            (name = "health", description = "Health check endpoints."),
            (name = "api_clusters", description = "API endpoints used by the front."),
            (name = "proxy_clusters", description = "Proxy cluster endpoints."),
            (name = "auth_clusters", description = "Authentication endpoints for clusters."),
        ),
)]
pub struct ApiDoc;
