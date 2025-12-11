use thiserror::Error;

#[derive(Debug, Error)]
pub enum CliConfigError {
    #[error("CLIERROR0001: Invalid server URL provided: {0} with error: {1}")]
    InvalidServerUrl(String, String),
    #[error("CLIERROR0002: Server '{0}' not found in configuration")]
    ServerNotFound(String),
    #[error("CLIERROR0003: YAML Parse Error: {0}")]
    YamlParseError(String),
    #[error("CLIERROR0004: YAML Serialize Error: {0}")]
    YamlSerializeError(String),
}
