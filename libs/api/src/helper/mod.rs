use actix_web::{http::header::ContentType, HttpRequest, HttpResponse};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AuthError {
    #[error("Invalid token")]
    InvalidToken,
    #[error("Invalid token: {0}")]
    InvalidTokenDetail(String),
    #[error("No Authorization header found")]
    NoToken,
}

impl AuthError {
    pub fn into_http_response(&self) -> HttpResponse {
        match self {
            AuthError::InvalidToken => HttpResponse::Unauthorized()
                .content_type(ContentType::plaintext())
                .body("Invalid token"),
            AuthError::NoToken => HttpResponse::Unauthorized()
                .content_type(ContentType::plaintext())
                .body("No Authorization header found"),
            AuthError::InvalidTokenDetail(detail) => HttpResponse::Unauthorized()
                .content_type(ContentType::plaintext())
                .body(format!("Invalid token: {}", detail)),
        }
    }

    pub fn into_actix_error(&self) -> actix_web::Error {
        match self {
            AuthError::InvalidToken => actix_web::error::ErrorUnauthorized("Invalid token"),
            AuthError::NoToken => {
                actix_web::error::ErrorUnauthorized("No Authorization header found")
            }
            AuthError::InvalidTokenDetail(detail) => {
                actix_web::error::ErrorUnauthorized(format!("Invalid token: {}", detail))
            }
        }
    }
}

pub fn extract_authorization_header(req: &HttpRequest) -> Result<&str, AuthError> {
    let header = req.headers();
    let token = match header.get("Authorization") {
        Some(token) => match token.to_str() {
            Ok(token) => {
                if let Some(end) = token.strip_prefix("Bearer ") {
                    end
                } else {
                    return Err(AuthError::InvalidToken);
                }
            }
            Err(err) => {
                return Err(AuthError::InvalidTokenDetail(err.to_string()));
            }
        },
        None => {
            return Err(AuthError::NoToken);
        }
    };
    Ok(token)
}
