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
    #[must_use]
    pub fn new() -> Self {
        CliClusterConfig { token_exist: true }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_cluster_config_records_that_a_token_exists() {
        // A cluster entry is only written once its token has been stored, so
        // `new` (and the `Default` that forwards to it) start out at `true`.
        assert!(CliClusterConfig::new().token_exist);
        assert!(CliClusterConfig::default().token_exist);
    }
}
