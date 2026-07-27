use std::vec;

use kube::config::NamedContext;
use serde::{Deserialize, Serialize};

use crate::{cli_config::CliConfig, ctx::CliCtx, output::TableRow};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GetContextOutput {
    pub current_context: bool,
    pub name: String,
    pub cluster: String,
    pub auth_info: String,
    pub namespace: Option<String>,
    pub is_proxy_auth: bool,
    pub proxy_server_url: Option<String>,
    pub proxy_namespace: Option<String>,
    pub proxy_name: Option<String>,
}

impl GetContextOutput {
    #[must_use]
    pub fn new_from_kubeconfig(ctx: &NamedContext, cli_ctx: CliCtx) -> Option<GetContextOutput> {
        let cluster = cli_ctx.kubeconfig.clusters.iter().find(|c| {
            ctx.context
                .as_ref()
                .is_some_and(|context| context.cluster == c.name)
        })?;
        let context = ctx.context.as_ref()?;
        let url_info = CliConfig::proxy_url_to_tuple(
            &cluster
                .cluster
                .clone()
                .unwrap_or_default()
                .server
                .unwrap_or_default(),
        )
        .unwrap_or_default();
        Some(GetContextOutput {
            current_context: match &cli_ctx.kubeconfig.current_context {
                Some(current) => current == &ctx.name,
                None => false,
            },
            name: ctx.name.clone(),
            cluster: cluster.name.clone(),
            auth_info: context.user.clone().unwrap_or(String::new()),
            namespace: context.namespace.clone(),
            is_proxy_auth: !url_info.cluster_name.is_empty(),
            proxy_server_url: Some(url_info.server_name),
            proxy_namespace: Some(url_info.namespace),
            proxy_name: Some(url_info.cluster_name),
        })
    }
}

impl TableRow for GetContextOutput {
    fn headers() -> Vec<String> {
        vec![
            "CURRENT".to_string(),
            "NAME".to_string(),
            "CLUSTER".to_string(),
            "AUTHINFO".to_string(),
            "NAMESPACE".to_string(),
            "IS PROXY AUTH".to_string(),
            "PROXY SERVER URL".to_string(),
            "PROXY NAMESPACE".to_string(),
            "PROXY NAME".to_string(),
        ]
    }

    fn row(&self) -> Vec<String> {
        vec![
            if self.current_context {
                "*".to_string()
            } else {
                String::new()
            },
            self.name.clone(),
            self.cluster.clone(),
            self.auth_info.clone(),
            self.namespace.clone().unwrap_or_default(),
            if self.is_proxy_auth {
                "Yes".to_string()
            } else {
                "No".to_string()
            },
            self.proxy_server_url.clone().unwrap_or_default(),
            self.proxy_namespace.clone().unwrap_or_default(),
            self.proxy_name.clone().unwrap_or_default(),
        ]
    }
}
