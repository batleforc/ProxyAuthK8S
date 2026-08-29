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

#[cfg(test)]
mod tests {
    use super::*;
    use kube::config::{Cluster, Context, NamedCluster};

    fn named_cluster(name: &str, server: &str) -> NamedCluster {
        NamedCluster {
            name: name.to_string(),
            cluster: Some(Cluster {
                server: Some(server.to_string()),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn named_context(name: &str, cluster: &str, namespace: Option<&str>) -> NamedContext {
        NamedContext {
            name: name.to_string(),
            context: Some(Context {
                cluster: cluster.to_string(),
                user: Some("tester".to_string()),
                namespace: namespace.map(std::string::ToString::to_string),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    /// A context with one proxy-auth cluster and one plain cluster, the proxy
    /// one being current.
    fn cli_ctx() -> CliCtx {
        let mut cli_ctx = CliCtx::for_test();
        cli_ctx.kubeconfig.clusters = vec![
            named_cluster("proxy", "https://localhost:5437/clusters/team-a/prod"),
            named_cluster("plain", "https://k8s.example.com"),
        ];
        cli_ctx.kubeconfig.contexts = vec![
            named_context("proxy-ctx", "proxy", Some("apps")),
            named_context("plain-ctx", "plain", None),
        ];
        cli_ctx.kubeconfig.current_context = Some("proxy-ctx".to_string());
        cli_ctx
    }

    #[test]
    fn a_proxy_auth_context_is_split_into_its_server_namespace_and_cluster() {
        let cli_ctx = cli_ctx();
        let output =
            GetContextOutput::new_from_kubeconfig(&cli_ctx.kubeconfig.contexts[0], cli_ctx.clone())
                .expect("the context resolves to a cluster");

        assert!(output.current_context);
        assert_eq!(output.name, "proxy-ctx");
        assert_eq!(output.cluster, "proxy");
        assert_eq!(output.auth_info, "tester");
        assert_eq!(output.namespace.as_deref(), Some("apps"));
        assert!(output.is_proxy_auth);
        // The proxy server is reported by its derived name, not the raw URL.
        assert_eq!(output.proxy_server_url.as_deref(), Some("localhost-5437"));
        assert_eq!(output.proxy_namespace.as_deref(), Some("team-a"));
        assert_eq!(output.proxy_name.as_deref(), Some("prod"));
    }

    #[test]
    fn a_plain_cluster_url_is_not_reported_as_proxy_auth() {
        let cli_ctx = cli_ctx();
        let output =
            GetContextOutput::new_from_kubeconfig(&cli_ctx.kubeconfig.contexts[1], cli_ctx.clone())
                .expect("the context resolves to a cluster");

        // The URL carries no /clusters/<ns>/<name>, so the proxy fields stay empty.
        assert!(!output.current_context);
        assert!(!output.is_proxy_auth);
        assert_eq!(output.proxy_server_url.as_deref(), Some(""));
        assert_eq!(output.proxy_namespace.as_deref(), Some(""));
        assert_eq!(output.proxy_name.as_deref(), Some(""));
        assert!(output.namespace.is_none());
    }

    #[test]
    fn a_context_pointing_at_an_unknown_cluster_is_dropped() {
        let cli_ctx = cli_ctx();
        let dangling = named_context("dangling-ctx", "missing", None);
        assert!(GetContextOutput::new_from_kubeconfig(&dangling, cli_ctx.clone()).is_none());

        // So is a named context with no context body at all.
        let empty = NamedContext {
            name: "empty-ctx".to_string(),
            context: None,
            ..Default::default()
        };
        assert!(GetContextOutput::new_from_kubeconfig(&empty, cli_ctx).is_none());
    }

    #[test]
    fn nothing_is_current_when_the_kubeconfig_has_no_current_context() {
        let mut cli_ctx = cli_ctx();
        cli_ctx.kubeconfig.current_context = None;
        let output =
            GetContextOutput::new_from_kubeconfig(&cli_ctx.kubeconfig.contexts[0], cli_ctx.clone())
                .expect("the context resolves to a cluster");
        assert!(!output.current_context);
    }

    #[test]
    fn a_row_lines_up_with_the_headers() {
        let cli_ctx = cli_ctx();
        let output =
            GetContextOutput::new_from_kubeconfig(&cli_ctx.kubeconfig.contexts[0], cli_ctx.clone())
                .expect("the context resolves to a cluster");

        assert_eq!(
            GetContextOutput::headers(),
            vec![
                "CURRENT",
                "NAME",
                "CLUSTER",
                "AUTHINFO",
                "NAMESPACE",
                "IS PROXY AUTH",
                "PROXY SERVER URL",
                "PROXY NAMESPACE",
                "PROXY NAME",
            ]
        );
        assert_eq!(
            output.row(),
            vec![
                "*",
                "proxy-ctx",
                "proxy",
                "tester",
                "apps",
                "Yes",
                "localhost-5437",
                "team-a",
                "prod",
            ]
        );
        assert_eq!(GetContextOutput::headers().len(), output.row().len());
    }

    #[test]
    fn the_flag_columns_and_the_optional_ones_render_their_empty_forms() {
        let cli_ctx = cli_ctx();
        let output =
            GetContextOutput::new_from_kubeconfig(&cli_ctx.kubeconfig.contexts[1], cli_ctx.clone())
                .expect("the context resolves to a cluster");
        let row = output.row();

        // Not current -> blank marker; not proxy-auth -> "No"; None -> "".
        assert_eq!(row[0], "");
        assert_eq!(row[5], "No");
        assert_eq!(row[4], "");
    }
}
