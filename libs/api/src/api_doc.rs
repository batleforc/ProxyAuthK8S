use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(description = "Reverse Proxy K8s Api"),
    tags(
            (name = "health", description = "Health check endpoints."),
            (name = "api_clusters", description = "API endpoints used by the front."),
            (name = "proxy_clusters", description = "Proxy cluster endpoints."),
            (name = "auth_clusters", description = "Authentication endpoints for clusters."),
        ),
)]
pub struct ApiDoc;
