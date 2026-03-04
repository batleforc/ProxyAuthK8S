use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Model for the callback response after successful authentication with the cluster.
///
/// This model contains the access token, refresh token, cluster URL, subject, and ID token returned by the cluster after successful authentication.
#[derive(Deserialize, Serialize, ToSchema)]
pub struct CallbackModel {
    pub access_token: String,
    pub refresh_token: String,
    pub cluster_url: String,
    pub subject: String,
    pub id_token: String,
}
