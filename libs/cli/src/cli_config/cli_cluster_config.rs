use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CliClusterConfig {
    pub token_exist: bool,
}

impl Default for CliClusterConfig {
    fn default() -> Self {
        CliClusterConfig::new()
    }
}

impl CliClusterConfig {
    pub fn new() -> Self {
        CliClusterConfig { token_exist: true }
    }
}
