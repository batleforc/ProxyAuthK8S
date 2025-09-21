use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(description = "Reverse Proxy K8s Api"),
    tags(
            (name = "health", description = "Health check endpoints.")
        ),
)]
pub struct ApiDoc;
