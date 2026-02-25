use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

//use super::allowed_crd_configuration::AllowedCrdConfiguration;
use super::allowed_path_configuration::AllowedPathConfiguration;

/// Enum of the allowed paths configuration, currently only supports path and crd, but can be extended in the future
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub enum AllowedPathConfigurationEnum {
    Path(AllowedPathConfiguration),
    // TODO : implement Crd(AllowedCrdConfiguration),
}
