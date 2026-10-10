//! The command context: everything a handler needs, resolved once at startup.
//!
//! [`CliCtx`] is built from the parsed [`Cli`] arguments (see `build.rs`) and
//! then threaded through every command handler.

use std::{
    env,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

use cli_trace::level::VerboseLevel;
use kube::config::Kubeconfig;
use serde::{Deserialize, Serialize};

use crate::{
    cli_config::{CliConfig, browser::BrowserFlag},
    error::ProxyAuthK8sError,
};

mod build;
pub mod format;

pub use format::ContextFormat;

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
    /// `login --browser` for this run (not part of any saved state).
    #[serde(skip)]
    pub browser_flag: Option<BrowserFlag>,
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
        let yaml_content = serde_yaml_ng::to_string(&file)
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
impl CliCtx {
    /// A context with nothing resolved from disk: enough to exercise the pure
    /// accessors and the renderers without touching the filesystem.
    pub(crate) fn for_test() -> Self {
        CliCtx {
            namespace: String::new(),
            kubeconfig_path: PathBuf::new(),
            kubeconfig: Kubeconfig::default(),
            context: None,
            verbose: None,
            server_url: String::new(),
            format: ContextFormat::default(),
            invoked_from_kubectl: false,
            config: CliConfig::new(),
            config_path: PathBuf::new(),
            browser_flag: None,
        }
    }

    /// Like [`CliCtx::for_test`], but with its kubeconfig (created empty) and
    /// CLI config paths inside `dir`, for tests that write them.
    pub(crate) fn for_test_in(dir: &Path) -> Self {
        let kubeconfig_path = dir.join("kubeconfig");
        fs::write(&kubeconfig_path, "").unwrap();
        CliCtx {
            kubeconfig_path,
            config_path: dir.join("proxyauth_config.yaml"),
            ..CliCtx::for_test()
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
            browser_flag: None,
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

    #[test]
    fn kubeconfig_env_with_only_empty_entries_falls_back_to_home() {
        let paths = CliCtx::detect_kubeconfig_paths(
            None,
            Some(env::join_paths(["", ""]).unwrap()),
            Some("/home/u".to_string()),
        );
        assert_eq!(paths, [PathBuf::from("/home/u/.kube/config")]);
    }

    #[test]
    fn verbose_count_maps_onto_the_tracing_level() {
        let mut ctx = CliCtx::for_test();
        // No -v at all, and an explicit 0, both mean INFO.
        assert_eq!(ctx.to_tracing_verbose_level(), VerboseLevel::INFO);
        ctx.verbose = Some(0);
        assert_eq!(ctx.to_tracing_verbose_level(), VerboseLevel::INFO);
        ctx.verbose = Some(1);
        assert_eq!(ctx.to_tracing_verbose_level(), VerboseLevel::DEBUG);
        ctx.verbose = Some(2);
        assert_eq!(ctx.to_tracing_verbose_level(), VerboseLevel::TRACE);
        // Anything past -vv saturates at TRACE rather than wrapping around.
        ctx.verbose = Some(3);
        assert_eq!(ctx.to_tracing_verbose_level(), VerboseLevel::TRACE);
        ctx.verbose = Some(u8::MAX);
        assert_eq!(ctx.to_tracing_verbose_level(), VerboseLevel::TRACE);
    }
}
