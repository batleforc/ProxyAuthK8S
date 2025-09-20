use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(description = "Reverse Proxy K8s Api"),
)]
pub struct ApiDoc;