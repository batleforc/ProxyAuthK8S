use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::default::default_disabled;

/// Allowed path configuration, used in conjunction with the allowed_paths configuration
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct AllowedPathConfiguration {
    /// The path to allow, if the request path equals this path, it will be allowed
    pub path: String,

    /// Wether or not the path is parametised
    /// if true, the path will be treated as a template, it either handle wildcard parameters, like "*" or "dev-*", or mustache-like parameters, like "{{username}}" or "{{group}}"
    /// for example, if the path is "/api/v1/namespaces/*/pods", it will allow all requests to pods in any namespace
    /// if the path is "/api/v1/namespaces/dev-*/pods", it will allow all requests to pods in namespaces that start with "dev-"
    /// It will also try to detect mustache-like parameters, for example, if the path is "/api/v1/namespaces/{{username}}/pods"
    /// it will allow all requests to pods in namespaces carrying the username as a parameter
    /// if the selected field is an array, like the groups claim, it will try to match any of the values in the array
    /// for example, if the groups claim is ["dev-alice", "dev-bob"] and the path is "/api/v1/namespaces/{{group}}/pods", it will allow all requests to pods in namespaces that match either "dev-alice" or "dev-bob"
    /// Allowed parameters are : {{username}} and {{group}}
    /// default: false
    #[serde(default = "default_disabled")]
    pub parametised: bool,
}

impl AllowedPathConfiguration {
    pub fn validate(&self) -> Result<(), String> {
        if self.parametised {
            // detect any mustache-like parameters
            let mustache_regex = regex::Regex::new(r"\{\{(\w+)\}\}").unwrap();
            for cap in mustache_regex.captures_iter(&self.path) {
                let param = &cap[1];
                if param != "username" && param != "group" {
                    return Err(format!("Invalid parameter in path: {}, allowed parameters are {{username}} and {{group}}", param));
                }
            }
        }
        Ok(())
    }

    pub fn extract_parameters(&self) -> Vec<String> {
        let mut params = Vec::new();
        if self.parametised {
            let mustache_regex = regex::Regex::new(r"\{\{(\w+)\}\}").unwrap();
            for cap in mustache_regex.captures_iter(&self.path) {
                let param = &cap[1];
                params.push(param.to_string());
            }
        }
        params
    }

    pub fn to_possible_paths(&self, username: &str, groups: &[String]) -> Vec<String> {
        if self.parametised {
            let mut paths = Vec::new();
            let mut path = self.path.clone();
            if path.contains("{{username}}") {
                path = path.replace("{{username}}", username);
            }
            if path.contains("{{group}}") {
                for group in groups {
                    paths.push(path.replace("{{group}}", group));
                }
            } else {
                paths.push(path);
            }
            paths
        } else {
            vec![self.path.clone()]
        }
    }

    pub fn has_wildcard(&self) -> bool {
        self.path.contains('*')
    }
}
