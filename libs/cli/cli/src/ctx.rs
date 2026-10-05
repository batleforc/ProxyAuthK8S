use clap::ValueEnum;
use cli_trace::level::VerboseLevel;
use kube::config::Kubeconfig;
use serde::{Deserialize, Serialize};
use std::{
    env,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

use crate::{cli_config::CliConfig, error::ProxyAuthK8sError};

#[derive(Serialize, Deserialize, Debug, Clone, Default, ValueEnum)]
pub enum ContextFormat {
    #[default]
    Table,
    Json,
    Yaml,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CliCtx {
    pub namespace: String,
    pub kubeconfig_path: PathBuf,
    pub kubeconfig: Kubeconfig,
    pub context: Option<String>,
    pub verbose: Option<u8>,
    pub server_url: String,
    pub format: ContextFormat,
    pub invoked_from_kubectl: bool,
    pub config: CliConfig,
    pub config_path: PathBuf,
}

impl TryFrom<super::Cli> for CliCtx {
    type Error = ProxyAuthK8sError;

    /// Build the context, surfacing every failure as a clean error instead of a
    /// panic. This matters because the CLI is also a `kubectl` exec-credential
    /// plugin: a panic would emit a Rust backtrace and a non-protocol exit,
    /// breaking `kubectl` auth with an opaque crash on a merely malformed config.
    fn try_from(cli: super::Cli) -> Result<Self, Self::Error> {
        let kubeconfig_paths = CliCtx::detect_kubeconfig_paths(
            cli.kubeconfig,
            env::var_os("KUBECONFIG"),
            env::var("HOME").ok(),
        );
        let kubeconfig_path = CliCtx::kubeconfig_write_target(&kubeconfig_paths)
            .ok_or(ProxyAuthK8sError::KubeconfigPathCouldNotBeCalculated)?;

        if !kubeconfig_path.exists() {
            // If the kubeconfig file does not exist, create an empty one (0600).
            crate::helper::secure_write(&kubeconfig_path, "").map_err(|e| {
                ProxyAuthK8sError::KubeconfigWriteError(format!(
                    "Failed to create kubeconfig file at {}: {}",
                    kubeconfig_path.to_string_lossy(),
                    e
                ))
            })?;
        }
        // Like kubectl: merge every listed file that exists, first file wins.
        let kubeconfig = kubeconfig_paths
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
            })?;
        let invoked_from_kubectl = env::args().next().is_some_and(|arg0| {
            PathBuf::from(arg0)
                .file_stem()
                .is_some_and(|stem| stem == "kubectl")
        });
        // Load CLI configuration
        let config_path = if let Some(path) = cli.proxy_auth_config {
            path
        } else {
            let home_env = env::var("HOME").unwrap_or_default();
            if home_env.is_empty() {
                return Err(ProxyAuthK8sError::ConfigPathCouldNotBeCalculated);
            }
            PathBuf::from(format!("{home_env}/.kube/proxyauth_config.yaml"))
        };
        let config = if config_path.exists() {
            CliConfig::read_from_file(config_path.clone()).map_err(|e| {
                ProxyAuthK8sError::KubeconfigReadError(format!(
                    "Failed to read config file at {}: {}",
                    config_path.to_string_lossy(),
                    e
                ))
            })?
        } else {
            CliConfig::default()
                .write_to_file(config_path.clone())
                .cloned()
                .map_err(|e| {
                    ProxyAuthK8sError::KubeconfigWriteError(format!(
                        "Failed to create default config file at {}: {}",
                        config_path.to_string_lossy(),
                        e
                    ))
                })?
        };

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
    /// The kubeconfig files to read, in kubectl's order of precedence:
    /// `--kubeconfig` (a single file), else every entry of `$KUBECONFIG`
    /// (split like `$PATH`, empty entries ignored), else `$HOME/.kube/config`.
    #[must_use]
    pub fn detect_kubeconfig_paths(
        explicit: Option<PathBuf>,
        kubeconfig_env: Option<OsString>,
        home: Option<String>,
    ) -> Vec<PathBuf> {
        if let Some(path) = explicit {
            return vec![path];
        }
        if let Some(value) = kubeconfig_env {
            let paths: Vec<PathBuf> = env::split_paths(&value)
                .filter(|p| !p.as_os_str().is_empty())
                .collect();
            if !paths.is_empty() {
                return paths;
            }
        }
        match home {
            Some(home) if !home.is_empty() => vec![PathBuf::from(format!("{home}/.kube/config"))],
            _ => vec![],
        }
    }

    /// The file the CLI writes to: the first one that exists, else the first one
    /// (it will be created). Being first in the merge order, whatever is written
    /// there takes precedence over the same entry in a later file.
    #[must_use]
    pub fn kubeconfig_write_target(paths: &[PathBuf]) -> Option<PathBuf> {
        paths
            .iter()
            .find(|p| p.exists())
            .or_else(|| paths.first())
            .cloned()
    }

    fn read_kubeconfig_file(path: &Path) -> Result<Kubeconfig, ProxyAuthK8sError> {
        let content = fs::read_to_string(path).map_err(|e| {
            ProxyAuthK8sError::KubeconfigReadError(format!(
                "Failed to read kubeconfig file at {}: {}",
                path.to_string_lossy(),
                e
            ))
        })?;
        Kubeconfig::from_yaml(&content).map_err(|e| {
            ProxyAuthK8sError::KubeconfigReadError(format!(
                "Failed to parse kubeconfig file at {}: {}",
                path.to_string_lossy(),
                e
            ))
        })
    }

    /// Apply `edit` to the kubeconfig file the CLI writes to, then save it.
    ///
    /// Only that file is rewritten, from its own content: the merged view in
    /// `self.kubeconfig` spans every `$KUBECONFIG` file, and writing it back
    /// would copy the other files' entries into this one. The same edit is
    /// applied to the merged view so it stays current for the rest of the run.
    pub fn edit_kubeconfig<R>(
        &mut self,
        edit: impl Fn(&mut Kubeconfig) -> R,
    ) -> Result<R, ProxyAuthK8sError> {
        let mut file = Self::read_kubeconfig_file(&self.kubeconfig_path)?;
        let result = edit(&mut file);
        let yaml_content = serde_yaml::to_string(&file)
            .map_err(|e| ProxyAuthK8sError::YamlSerializeError(e.to_string()))?;
        crate::helper::secure_write(&self.kubeconfig_path, &yaml_content)
            .map_err(|e| ProxyAuthK8sError::KubeconfigWriteError(e.to_string()))?;
        edit(&mut self.kubeconfig);
        Ok(result)
    }

    #[must_use]
    pub fn to_tracing_verbose_level(&self) -> VerboseLevel {
        match self.verbose.unwrap_or(0) {
            0 => VerboseLevel::INFO,
            1 => VerboseLevel::DEBUG,
            2 => VerboseLevel::TRACE,
            _ => VerboseLevel::TRACE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir(test: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!("proxyauth-cli-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn explicit_kubeconfig_wins_over_env_and_home() {
        let paths = CliCtx::detect_kubeconfig_paths(
            Some(PathBuf::from("/explicit")),
            Some(OsString::from("/a:/b")),
            Some("/home/u".to_string()),
        );
        assert_eq!(paths, [PathBuf::from("/explicit")]);
    }

    #[test]
    fn kubeconfig_env_is_split_like_path_and_skips_empty_entries() {
        let paths = CliCtx::detect_kubeconfig_paths(
            None,
            Some(env::join_paths(["/a", "", "/b"]).unwrap()),
            Some("/home/u".to_string()),
        );
        assert_eq!(paths, [PathBuf::from("/a"), PathBuf::from("/b")]);
    }

    #[test]
    fn falls_back_to_home_then_to_nothing() {
        assert_eq!(
            CliCtx::detect_kubeconfig_paths(None, Some(OsString::new()), Some("/home/u".into())),
            [PathBuf::from("/home/u/.kube/config")]
        );
        assert!(CliCtx::detect_kubeconfig_paths(None, None, None).is_empty());
        assert!(CliCtx::detect_kubeconfig_paths(None, None, Some(String::new())).is_empty());
    }

    #[test]
    fn write_target_is_the_first_existing_file_else_the_first_one() {
        let dir = scratch_dir("target");
        let (missing, existing, other) = (dir.join("missing"), dir.join("a"), dir.join("b"));
        fs::write(&existing, "").unwrap();
        fs::write(&other, "").unwrap();

        assert_eq!(
            CliCtx::kubeconfig_write_target(&[missing.clone(), existing.clone(), other]),
            Some(existing)
        );
        assert_eq!(
            CliCtx::kubeconfig_write_target(std::slice::from_ref(&missing)),
            Some(missing)
        );
        assert_eq!(CliCtx::kubeconfig_write_target(&[]), None);
    }

    #[test]
    fn edit_kubeconfig_only_rewrites_the_target_file() {
        let dir = scratch_dir("edit");
        let (first, second) = (dir.join("first"), dir.join("second"));
        fs::write(&first, "apiVersion: v1\nkind: Config\ncurrent-context: one\ncontexts:\n- name: one\n  context:\n    cluster: c1\n").unwrap();
        let second_content =
            "apiVersion: v1\nkind: Config\ncontexts:\n- name: two\n  context:\n    cluster: c2\n";
        fs::write(&second, second_content).unwrap();

        let mut ctx = CliCtx {
            namespace: String::new(),
            kubeconfig_path: first.clone(),
            kubeconfig: CliCtx::read_kubeconfig_file(&first)
                .unwrap()
                .merge(CliCtx::read_kubeconfig_file(&second).unwrap())
                .unwrap(),
            context: None,
            verbose: None,
            server_url: String::new(),
            format: ContextFormat::Table,
            invoked_from_kubectl: false,
            config: CliConfig::default(),
            config_path: dir.join("proxyauth_config.yaml"),
        };
        assert_eq!(ctx.kubeconfig.contexts.len(), 2);

        ctx.edit_kubeconfig(|kubeconfig| kubeconfig.current_context = Some("two".to_string()))
            .unwrap();

        let written = CliCtx::read_kubeconfig_file(&first).unwrap();
        assert_eq!(written.current_context.as_deref(), Some("two"));
        // The other file's context was not copied into the target file...
        assert_eq!(written.contexts.len(), 1);
        // ...and that file is untouched.
        assert_eq!(fs::read_to_string(&second).unwrap(), second_content);
        // The merged view follows the edit.
        assert_eq!(ctx.kubeconfig.current_context.as_deref(), Some("two"));
    }
}
