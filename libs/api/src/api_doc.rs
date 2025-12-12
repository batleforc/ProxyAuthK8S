use utoipa::{
    openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme},
    Modify, OpenApi,
};

pub struct SecurityAddons;

impl Modify for SecurityAddons {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.as_mut().unwrap(); // we can unwrap safely since there already is components registered.
        components.add_security_scheme(
            "bearer_auth",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .bearer_format("JWT")
                    .build(),
            ),
        )
    }
}

#[derive(OpenApi)]
#[openapi(
    info(
      title = "ProxyAuthK8S",
      description = "Reverse Proxy K8s Api",
      contact(
            name = "Batleforc",
            email = "maxleriche.60@gmail.com",
            url = "https://maxleriche.net"
        ),
      license(
            name = "MIT",
            url = "https://opensource.org/license/mit/"
        ),
      ),
    tags(
            (name = "health", description = "Health check endpoints."),
            (name = "api_clusters", description = "API endpoints used by the front."),
            (name = "proxy_clusters", description = "Proxy cluster endpoints."),
            (name = "auth_clusters", description = "Authentication endpoints for clusters."),
        ),
    modifiers(&SecurityAddons)
)]
pub struct ApiDoc;
