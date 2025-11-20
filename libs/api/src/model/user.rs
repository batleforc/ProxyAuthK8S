use actix_web::FromRequest;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct User {
    pub username: String,
    pub groups: Vec<String>,
}

impl FromRequest for User {
    type Error = actix_web::Error;
    type Future = std::pin::Pin<Box<dyn std::future::Future<Output = Result<Self, Self::Error>>>>;

    // https://github.com/batleforc/rust-template/blob/main/src/model/user.rs
    fn from_request(
        req: &actix_web::HttpRequest,
        _payload: &mut actix_web::dev::Payload,
    ) -> Self::Future {
        let req = req.clone();
        tracing::info!("Start auth middleware");
        Box::pin(async move {
            // Get authorization header from request
            let auth_header = req.headers().get("Authorization").cloned();
            if auth_header.is_none() {
                tracing::warn!("No Authorization header found");
                return Err(actix_web::error::ErrorUnauthorized(
                    "No Authorization header",
                ));
            }
            let auth_header = auth_header.unwrap();
            let auth_str = auth_header
                .to_str()
                .map_err(|_| actix_web::error::ErrorUnauthorized("Invalid Authorization header"))?;
            if !auth_str.starts_with("Bearer ") {
                tracing::warn!("Invalid Authorization header format");
                return Err(actix_web::error::ErrorUnauthorized(
                    "Invalid Authorization header",
                ));
            }
            let token = auth_str.trim_start_matches("Bearer ").to_string();
            // In a real implementation, extract user info from request (e.g., headers, tokens)
            // Here we return a dummy user for illustration
            let user = User {
                username: "dummy_user".to_string(),
                groups: vec!["group1".to_string(), "group2".to_string()],
            };
            Ok(user)
        })
    }
}
