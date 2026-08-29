//! The command context: everything a handler needs, resolved once at startup.
//!
//! [`CliCtx`] is built from the parsed [`Cli`] arguments (see `build.rs`) and
//! then threaded through every command handler.

use std::{env, path::PathBuf};

use cli_trace::level::VerboseLevel;
use kube::config::Kubeconfig;
use serde::{Deserialize, Serialize};

use crate::{cli_config::CliConfig, error::ProxyAuthK8sError};

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
}

impl CliCtx {
    #[must_use]
    pub fn detect_kubeconfig_path(kubeconfig: Option<String>) -> Option<String> {
        if let Some(path) = kubeconfig {
            Some(path)
        } else if let Ok(env_path) = env::var("KUBECONFIG") {
            Some(env_path)
        } else {
            let home_env = env::var("HOME").unwrap_or_default();
            if home_env.is_empty() {
                None
            } else {
                Some(format!("{home_env}/.kube/config"))
            }
        }
    }

    pub fn write_kubeconfig(&self) -> Result<(), ProxyAuthK8sError> {
        let yaml_content = serde_yaml::to_string(&self.kubeconfig)
            .map_err(|e| ProxyAuthK8sError::YamlSerializeError(e.to_string()))?;
        crate::helper::secure_write(&self.kubeconfig_path, &yaml_content)
            .map_err(|e| ProxyAuthK8sError::KubeconfigWriteError(e.to_string()))
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
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, MutexGuard};

    use super::*;

    /// Serializes the tests that mutate the process environment. Under
    /// cargo-nextest each test is its own process, but a plain `cargo test`
    /// runs them as threads of one process, where concurrent `set_var`/`var`
    /// on the same variable would race.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Takes the environment lock, ignoring poisoning: a panicking test has
    /// already failed, and the guards below still restored what they changed.
    fn lock_env() -> MutexGuard<'static, ()> {
        ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Restores an environment variable to whatever it held before the test.
    struct EnvGuard {
        key: &'static str,
        previous: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let guard = EnvGuard {
                key,
                previous: env::var(key).ok(),
            };
            // SAFETY: the caller holds the environment lock (see `lock_env`).
            unsafe { env::set_var(key, value) };
            guard
        }

        fn unset(key: &'static str) -> Self {
            let guard = EnvGuard {
                key,
                previous: env::var(key).ok(),
            };
            // SAFETY: the caller holds the environment lock (see `lock_env`).
            unsafe { env::remove_var(key) };
            guard
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // SAFETY: the caller holds the environment lock (see `lock_env`).
            unsafe {
                match &self.previous {
                    Some(value) => env::set_var(self.key, value),
                    None => env::remove_var(self.key),
                }
            }
        }
    }

    #[test]
    fn detect_kubeconfig_path_prefers_the_explicit_flag() {
        // Declared first so it outlives the guards that restore the variables.
        let _env = lock_env();
        // The flag wins even when KUBECONFIG and HOME would both answer.
        let _kubeconfig = EnvGuard::set("KUBECONFIG", "/from/env/config");
        let _home = EnvGuard::set("HOME", "/home/tester");
        assert_eq!(
            CliCtx::detect_kubeconfig_path(Some("/from/flag/config".to_string())),
            Some("/from/flag/config".to_string())
        );
    }

    #[test]
    fn detect_kubeconfig_path_falls_back_to_the_kubeconfig_env_var() {
        // Declared first so it outlives the guards that restore the variables.
        let _env = lock_env();
        let _kubeconfig = EnvGuard::set("KUBECONFIG", "/from/env/config");
        assert_eq!(
            CliCtx::detect_kubeconfig_path(None),
            Some("/from/env/config".to_string())
        );
    }

    #[test]
    fn detect_kubeconfig_path_falls_back_to_home() {
        // Declared first so it outlives the guards that restore the variables.
        let _env = lock_env();
        let _kubeconfig = EnvGuard::unset("KUBECONFIG");
        let _home = EnvGuard::set("HOME", "/home/tester");
        assert_eq!(
            CliCtx::detect_kubeconfig_path(None),
            Some("/home/tester/.kube/config".to_string())
        );
    }

    #[test]
    fn detect_kubeconfig_path_gives_up_without_a_flag_env_or_home() {
        // Declared first so it outlives the guards that restore the variables.
        let _env = lock_env();
        let _kubeconfig = EnvGuard::unset("KUBECONFIG");
        // An empty HOME is treated the same as an unset one.
        let _home = EnvGuard::set("HOME", "");
        assert_eq!(CliCtx::detect_kubeconfig_path(None), None);
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
