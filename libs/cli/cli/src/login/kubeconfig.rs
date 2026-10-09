use kube::config::{
    AuthInfo, Cluster, Context, ExecConfig, ExecInteractiveMode, Kubeconfig, NamedAuthInfo,
    NamedCluster, NamedContext,
};

/// Names of the kubeconfig entries written for a proxied cluster.
///
/// The cluster and context names match the kubeconfig the dashboard hands out
/// (`ClusterCallbackView`), so logging in from the CLI after downloading it
/// from the UI updates those entries instead of duplicating them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyContextNames {
    pub cluster: String,
    pub user: String,
    pub context: String,
}

impl ProxyContextNames {
    #[must_use]
    pub fn new(namespace: &str, cluster: &str) -> Self {
        let base = format!("{namespace}-{cluster}");
        Self {
            user: format!("{base}-proxyauth"),
            context: format!("{base}-context"),
            cluster: base,
        }
    }
}

/// Add (or replace) the cluster, user and context entries pointing `kubectl`
/// at `<server_url>/clusters/<namespace>/<cluster>`, authenticated by
/// `kubectl proxyauth get-token`, and make that context the current one.
///
/// Entries are matched by name, so a second login refreshes them in place.
/// Unrelated entries are left untouched.
pub fn upsert_proxy_context(
    kubeconfig: &mut Kubeconfig,
    server_url: &str,
    namespace: &str,
    cluster: &str,
    certificate_authority_data: Option<&str>,
) -> ProxyContextNames {
    let names = ProxyContextNames::new(namespace, cluster);
    let server_url = server_url.trim_end_matches('/');

    upsert(
        &mut kubeconfig.clusters,
        |c| c.name == names.cluster,
        NamedCluster {
            name: names.cluster.clone(),
            cluster: Some(Cluster {
                server: Some(format!("{server_url}/clusters/{namespace}/{cluster}")),
                certificate_authority_data: certificate_authority_data.map(str::to_string),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    upsert(
        &mut kubeconfig.auth_infos,
        |a| a.name == names.user,
        NamedAuthInfo {
            name: names.user.clone(),
            auth_info: Some(AuthInfo {
                exec: Some(ExecConfig {
                    api_version: Some("client.authentication.k8s.io/v1".to_string()),
                    command: Some("kubectl".to_string()),
                    // `--flag=value` keeps each flag and its value in one argv:
                    // `-n <ns>` as a single argv would be parsed as " <ns>".
                    args: Some(vec![
                        "proxyauth".to_string(),
                        "get-token".to_string(),
                        format!("--namespace={namespace}"),
                        format!("--server-url={server_url}"),
                        cluster.to_string(),
                    ]),
                    // Required by kubectl for the v1 exec API. `get-token` only reads
                    // the keyring and never prompts.
                    interactive_mode: Some(ExecInteractiveMode::Never),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    upsert(
        &mut kubeconfig.contexts,
        |c| c.name == names.context,
        NamedContext {
            name: names.context.clone(),
            context: Some(Context {
                cluster: names.cluster.clone(),
                user: Some(names.user.clone()),
                ..Default::default()
            }),
            ..Default::default()
        },
    );

    kubeconfig.current_context = Some(names.context.clone());
    // A kubeconfig created empty by the CLI has neither field; kubectl expects them.
    kubeconfig
        .api_version
        .get_or_insert_with(|| "v1".to_string());
    kubeconfig.kind.get_or_insert_with(|| "Config".to_string());
    names
}

fn upsert<T>(entries: &mut Vec<T>, matches: impl Fn(&T) -> bool, entry: T) {
    match entries.iter_mut().find(|e| matches(e)) {
        Some(existing) => *existing = entry,
        None => entries.push(entry),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exec_args(kubeconfig: &Kubeconfig, user: &str) -> Vec<String> {
        kubeconfig
            .auth_infos
            .iter()
            .find(|a| a.name == user)
            .and_then(|a| a.auth_info.as_ref())
            .and_then(|a| a.exec.as_ref())
            .and_then(|e| e.args.clone())
            .expect("exec args are written")
    }

    #[test]
    fn writes_cluster_user_and_current_context_into_an_empty_kubeconfig() {
        let mut kubeconfig = Kubeconfig::default();

        let names = upsert_proxy_context(
            &mut kubeconfig,
            "https://proxy.example.com/",
            "team-a",
            "prod",
            None,
        );

        assert_eq!(names, ProxyContextNames::new("team-a", "prod"));
        assert_eq!(names.context, "team-a-prod-context");
        assert_eq!(
            kubeconfig.current_context.as_deref(),
            Some("team-a-prod-context")
        );
        assert_eq!(kubeconfig.api_version.as_deref(), Some("v1"));
        assert_eq!(kubeconfig.kind.as_deref(), Some("Config"));

        let server = kubeconfig.clusters[0]
            .cluster
            .as_ref()
            .and_then(|c| c.server.clone());
        assert_eq!(
            server.as_deref(),
            Some("https://proxy.example.com/clusters/team-a/prod")
        );
        assert_eq!(
            exec_args(&kubeconfig, &names.user),
            [
                "proxyauth",
                "get-token",
                "--namespace=team-a",
                "--server-url=https://proxy.example.com",
                "prod"
            ]
        );
        let context = kubeconfig.contexts[0].context.as_ref().unwrap();
        assert_eq!(context.cluster, names.cluster);
        assert_eq!(context.user.as_deref(), Some(names.user.as_str()));
    }

    #[test]
    fn a_second_login_replaces_entries_instead_of_duplicating_them() {
        let mut kubeconfig = Kubeconfig::default();
        upsert_proxy_context(
            &mut kubeconfig,
            "https://old.example.com",
            "team-a",
            "prod",
            None,
        );
        upsert_proxy_context(
            &mut kubeconfig,
            "https://new.example.com",
            "team-a",
            "prod",
            None,
        );

        assert_eq!(kubeconfig.clusters.len(), 1);
        assert_eq!(kubeconfig.auth_infos.len(), 1);
        assert_eq!(kubeconfig.contexts.len(), 1);
        assert!(
            exec_args(&kubeconfig, "team-a-prod-proxyauth")
                .contains(&"--server-url=https://new.example.com".to_string())
        );
    }

    #[test]
    fn unrelated_entries_are_kept() {
        let mut kubeconfig = Kubeconfig::from_yaml(
            r"
apiVersion: v1
kind: Config
clusters:
- name: other
  cluster:
    server: https://other.example.com
users:
- name: other
  user:
    token: abc
contexts:
- name: other
  context:
    cluster: other
    user: other
current-context: other
",
        )
        .unwrap();

        upsert_proxy_context(
            &mut kubeconfig,
            "https://proxy.example.com",
            "team-a",
            "prod",
            None,
        );

        assert_eq!(kubeconfig.clusters.len(), 2);
        assert_eq!(kubeconfig.auth_infos.len(), 2);
        assert_eq!(kubeconfig.contexts.len(), 2);
        assert!(kubeconfig.contexts.iter().any(|c| c.name == "other"));
        assert_eq!(
            kubeconfig.current_context.as_deref(),
            Some("team-a-prod-context")
        );
    }

    #[test]
    fn written_kubeconfig_round_trips_through_yaml() {
        let mut kubeconfig = Kubeconfig::default();
        upsert_proxy_context(
            &mut kubeconfig,
            "https://proxy.example.com",
            "team-a",
            "prod",
            None,
        );

        let yaml = serde_yaml_ng::to_string(&kubeconfig).unwrap();
        if let Ok(path) = std::env::var("PROXYAUTH_DUMP_KUBECONFIG") {
            std::fs::write(path, &yaml).unwrap();
        }
        let parsed = Kubeconfig::from_yaml(&yaml).unwrap();

        assert_eq!(
            parsed.current_context.as_deref(),
            Some("team-a-prod-context")
        );
        assert_eq!(
            exec_args(&parsed, "team-a-prod-proxyauth"),
            exec_args(&kubeconfig, "team-a-prod-proxyauth")
        );
    }

    #[test]
    fn writes_the_certificate_authority_when_known() {
        let mut kubeconfig = Kubeconfig::default();
        upsert_proxy_context(
            &mut kubeconfig,
            "https://proxy.example.com",
            "team-a",
            "prod",
            Some("Q0EgZGF0YQ=="),
        );
        let cluster = kubeconfig.clusters[0].cluster.as_ref().unwrap();
        assert_eq!(
            cluster.certificate_authority_data.as_deref(),
            Some("Q0EgZGF0YQ==")
        );

        upsert_proxy_context(
            &mut kubeconfig,
            "https://proxy.example.com",
            "team-a",
            "prod",
            None,
        );
        let cluster = kubeconfig.clusters[0].cluster.as_ref().unwrap();
        assert_eq!(cluster.certificate_authority_data, None);
    }
}
