//! Construction of the [`CliCtx`] from the parsed [`Cli`] arguments.
//!
//! Split out of `mod.rs` because it is the one place that touches the
//! filesystem: locating and reading the kubeconfig and the CLI config, and
//! creating either when missing.

use std::{
    env,
    path::{Path, PathBuf},
};

use kube::config::Kubeconfig;

use crate::{Cli, cli_config::CliConfig, ctx::CliCtx, error::ProxyAuthK8sError};

impl TryFrom<Cli> for CliCtx {
    type Error = ProxyAuthK8sError;

    /// Build the context, surfacing every failure as a clean error instead of a
    /// panic. This matters because the CLI is also a `kubectl` exec-credential
    /// plugin: a panic would emit a Rust backtrace and a non-protocol exit,
    /// breaking `kubectl` auth with an opaque crash on a merely malformed config.
    fn try_from(cli: Cli) -> Result<Self, Self::Error> {
        let kubeconfig_paths = CliCtx::detect_kubeconfig_paths(
            cli.kubeconfig,
            env::var_os("KUBECONFIG"),
            env::var("HOME").ok(),
        );
        let kubeconfig_path = CliCtx::kubeconfig_write_target(&kubeconfig_paths)
            .ok_or(ProxyAuthK8sError::KubeconfigPathCouldNotBeCalculated)?;
        CliCtx::ensure_kubeconfig_exists(&kubeconfig_path)?;
        let kubeconfig = CliCtx::load_merged_kubeconfig(&kubeconfig_paths)?;
        let invoked_from_kubectl = CliCtx::is_kubectl_invocation(env::args().next());
        // Load CLI configuration
        let config_path =
            CliCtx::resolve_config_path(cli.proxy_auth_config, env::var("HOME").ok())?;
        let config = CliCtx::load_or_create_config(&config_path)?;

        Ok(CliCtx {
            namespace: cli.namespace,
            kubeconfig,
            kubeconfig_path,
            context: cli.context,
            verbose: cli.verbose,
            server_url: cli.server_url,
            format: cli.format,
            invoked_from_kubectl,
            config,
            config_path,
        })
    }
}

impl CliCtx {
    /// If the kubeconfig file does not exist, create an empty one (0600).
    fn ensure_kubeconfig_exists(kubeconfig_path: &Path) -> Result<(), ProxyAuthK8sError> {
        if kubeconfig_path.exists() {
            return Ok(());
        }
        crate::helper::secure_write(kubeconfig_path, "").map_err(|e| {
            ProxyAuthK8sError::KubeconfigWriteError(format!(
                "Failed to create kubeconfig file at {}: {}",
                kubeconfig_path.to_string_lossy(),
                e
            ))
        })
    }

    /// Like kubectl: merge every listed file that exists, first file wins.
    fn load_merged_kubeconfig(paths: &[PathBuf]) -> Result<Kubeconfig, ProxyAuthK8sError> {
        paths
            .iter()
            .filter(|path| path.exists())
            .try_fold(Kubeconfig::default(), |merged, path| {
                merged
                    .merge(CliCtx::read_kubeconfig_file(path)?)
                    .map_err(|e| {
                        ProxyAuthK8sError::KubeconfigReadError(format!(
                            "Failed to merge kubeconfig file at {}: {}",
                            path.to_string_lossy(),
                            e
                        ))
                    })
            })
    }

    /// Whether the binary was started as `kubectl` (judged from `argv[0]`).
    fn is_kubectl_invocation(arg0: Option<String>) -> bool {
        arg0.is_some_and(|arg0| {
            PathBuf::from(arg0)
                .file_stem()
                .is_some_and(|stem| stem == "kubectl")
        })
    }

    /// `--proxy-auth-config`, else `$HOME/.kube/proxyauth_config.yaml`.
    fn resolve_config_path(
        explicit: Option<PathBuf>,
        home: Option<String>,
    ) -> Result<PathBuf, ProxyAuthK8sError> {
        if let Some(path) = explicit {
            return Ok(path);
        }
        let home_env = home.unwrap_or_default();
        if home_env.is_empty() {
            return Err(ProxyAuthK8sError::ConfigPathCouldNotBeCalculated);
        }
        Ok(PathBuf::from(format!(
            "{home_env}/.kube/proxyauth_config.yaml"
        )))
    }

    /// Read the CLI config at `config_path`, writing a default one there
    /// first if it does not exist.
    fn load_or_create_config(config_path: &Path) -> Result<CliConfig, ProxyAuthK8sError> {
        if config_path.exists() {
            CliConfig::read_from_file(config_path.to_path_buf()).map_err(|e| {
                ProxyAuthK8sError::KubeconfigReadError(format!(
                    "Failed to read config file at {}: {}",
                    config_path.to_string_lossy(),
                    e
                ))
            })
        } else {
            CliConfig::default()
                .write_to_file(config_path.to_path_buf())
                .cloned()
                .map_err(|e| {
                    ProxyAuthK8sError::KubeconfigWriteError(format!(
                        "Failed to create default config file at {}: {}",
                        config_path.to_string_lossy(),
                        e
                    ))
                })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn scratch_dir(test: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!("proxyauth-cli-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn kubectl_invocation_is_detected_from_argv0_stem() {
        assert!(CliCtx::is_kubectl_invocation(Some("kubectl".into())));
        assert!(CliCtx::is_kubectl_invocation(Some(
            "/usr/local/bin/kubectl".into()
        )));
        assert!(CliCtx::is_kubectl_invocation(Some("kubectl.exe".into())));
        assert!(!CliCtx::is_kubectl_invocation(Some(
            "/usr/bin/kubectl-proxyauth".into()
        )));
        assert!(!CliCtx::is_kubectl_invocation(Some(String::new())));
        assert!(!CliCtx::is_kubectl_invocation(None));
    }

    #[test]
    fn config_path_is_explicit_else_under_home() {
        assert_eq!(
            CliCtx::resolve_config_path(Some(PathBuf::from("/x/c.yaml")), None).unwrap(),
            PathBuf::from("/x/c.yaml")
        );
        assert_eq!(
            CliCtx::resolve_config_path(None, Some("/home/u".to_string())).unwrap(),
            PathBuf::from("/home/u/.kube/proxyauth_config.yaml")
        );
        assert!(matches!(
            CliCtx::resolve_config_path(None, None),
            Err(ProxyAuthK8sError::ConfigPathCouldNotBeCalculated)
        ));
        assert!(matches!(
            CliCtx::resolve_config_path(None, Some(String::new())),
            Err(ProxyAuthK8sError::ConfigPathCouldNotBeCalculated)
        ));
    }

    #[test]
    fn missing_config_is_created_with_defaults_then_read_back() {
        let dir = scratch_dir("config");
        let path = dir.join("proxyauth_config.yaml");

        let created = CliCtx::load_or_create_config(&path).unwrap();
        assert!(path.exists());
        assert!(created.servers.is_empty());
        assert!(created.default_server_name.is_empty());

        fs::write(&path, "default_server_name: a\nservers: {}\n").unwrap();
        let read = CliCtx::load_or_create_config(&path).unwrap();
        assert_eq!(read.default_server_name, "a");

        fs::write(&path, "not: [valid").unwrap();
        assert!(matches!(
            CliCtx::load_or_create_config(&path),
            Err(ProxyAuthK8sError::KubeconfigReadError(_))
        ));
    }

    #[test]
    fn missing_kubeconfig_is_created_empty_and_existing_one_is_kept() {
        let dir = scratch_dir("ensure");
        let path = dir.join("config");
        CliCtx::ensure_kubeconfig_exists(&path).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "");

        fs::write(&path, "kind: Config\n").unwrap();
        CliCtx::ensure_kubeconfig_exists(&path).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "kind: Config\n");
    }

    #[test]
    fn merged_kubeconfig_skips_missing_files_and_first_file_wins() {
        let dir = scratch_dir("merge");
        let (first, missing, second) = (dir.join("first"), dir.join("missing"), dir.join("second"));
        fs::write(&first, "apiVersion: v1\nkind: Config\ncurrent-context: one\ncontexts:\n- name: one\n  context:\n    cluster: c1\n").unwrap();
        fs::write(&second, "apiVersion: v1\nkind: Config\ncurrent-context: two\ncontexts:\n- name: two\n  context:\n    cluster: c2\n").unwrap();

        let merged = CliCtx::load_merged_kubeconfig(&[first, missing, second.clone()]).unwrap();
        assert_eq!(merged.current_context.as_deref(), Some("one"));
        assert_eq!(merged.contexts.len(), 2);

        assert!(
            CliCtx::load_merged_kubeconfig(&[dir.join("nope")])
                .unwrap()
                .contexts
                .is_empty()
        );

        fs::write(&second, "contexts: [unterminated").unwrap();
        assert!(matches!(
            CliCtx::load_merged_kubeconfig(&[second]),
            Err(ProxyAuthK8sError::KubeconfigReadError(_))
        ));
    }
}
