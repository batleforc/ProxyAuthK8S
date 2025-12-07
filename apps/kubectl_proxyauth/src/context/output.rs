use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GetContextOutput {
    pub current_context: bool,
    pub name: String,
    pub cluster: String,
    pub auth_info: String,
    pub namespace: Option<String>,
    pub is_proxy_auth: bool,
}
