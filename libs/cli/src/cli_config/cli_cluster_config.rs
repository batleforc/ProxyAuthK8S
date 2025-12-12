use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CliClusterConfig {
    pub token_exist: bool,
}

impl CliClusterConfig {
    pub fn new() -> Self {
        CliClusterConfig { token_exist: true }
    }
}
