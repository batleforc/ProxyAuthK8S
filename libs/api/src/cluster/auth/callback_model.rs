use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Deserialize, Serialize, ToSchema)]
pub struct CallbackModel {
    pub access_token: String,
    pub refresh_token: String,
    pub cluster_url: String,
    pub subject: String,
    pub id_token: String,
}
