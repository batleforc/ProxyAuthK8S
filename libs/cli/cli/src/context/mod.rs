use tracing::{debug, info};

use crate::{
    context::output::GetContextOutput, ctx::CliCtx, error::ProxyAuthK8sError, output::KubeList,
};

pub mod output;

impl CliCtx {
    pub fn handle_context(
        &mut self,
        context_name: Option<String>,
        list: bool,
        set: bool,
    ) -> Result<(), ProxyAuthK8sError> {
        if set {
            let Some(context_name) = context_name.clone() else {
                debug!("Context name must be provided when using the --set flag.");
                return Err(ProxyAuthK8sError::InvalidUsage(
                    "context name must be provided when using --set".to_string(),
                ));
            };
            // Find the context in the kubeconfig
            let context = self
                .kubeconfig
                .contexts
                .iter()
                .find(|ctx| ctx.name == context_name);
            if context.is_some() {
                // Set the current context
                info!("Setting current context to: {}", context_name);
                // Here you would implement the logic to actually set the context
                if let Err(e) = self.edit_kubeconfig(|kubeconfig| {
                    kubeconfig.current_context = Some(context_name.clone());
                }) {
                    debug!("Failed to write kubeconfig: {}", e);
                    return Err(e);
                }
            } else {
                debug!("Context '{}' not found in kubeconfig.", context_name);
                return Err(ProxyAuthK8sError::InvalidUsage(format!(
                    "context '{context_name}' not found in kubeconfig"
                )));
            }
        }
        let vec_context = if list {
            self.kubeconfig
                .contexts
                .iter()
                .filter_map(|ctx| GetContextOutput::new_from_kubeconfig(ctx, self.clone()))
                .collect::<Vec<output::GetContextOutput>>()
        } else if let Some(cluster_name) = context_name {
            let context = self
                .kubeconfig
                .contexts
                .iter()
                .find(|ctx| ctx.name == cluster_name);
            match context {
                Some(ctx) => match GetContextOutput::new_from_kubeconfig(ctx, self.clone()) {
                    Some(output) => vec![output],
                    None => vec![],
                },
                None => vec![],
            }
        } else {
            let context = self.kubeconfig.contexts.iter().find(|ctx| {
                ctx.name == self.kubeconfig.current_context.clone().unwrap_or_default()
            });
            match context {
                Some(ctx) => match GetContextOutput::new_from_kubeconfig(ctx, self.clone()) {
                    Some(output) => vec![output],
                    None => vec![],
                },
                None => vec![],
            }
        };
        let output = KubeList::new(vec_context);
        println!("{}", output.to_output(self.format.clone()));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kube::config::Kubeconfig;

    const KUBECONFIG: &str = "apiVersion: v1
kind: Config
clusters:
- name: c1
  cluster:
    server: https://proxy.example.com/clusters/team-a/prod
contexts:
- name: one
  context: {cluster: c1, user: u}
- name: two
  context: {cluster: c1, user: u}
current-context: one
users:
- name: u
  user: {}
";

    /// A context whose kubeconfig (on disk and in memory) holds `one` and
    /// `two`, `one` being current.
    fn ctx_in(dir: &std::path::Path) -> CliCtx {
        let mut ctx = CliCtx::for_test_in(dir);
        std::fs::write(&ctx.kubeconfig_path, KUBECONFIG).unwrap();
        ctx.kubeconfig = Kubeconfig::read_from(&ctx.kubeconfig_path).unwrap();
        ctx
    }

    fn current_on_disk(ctx: &CliCtx) -> Option<String> {
        Kubeconfig::read_from(&ctx.kubeconfig_path)
            .unwrap()
            .current_context
    }

    #[test]
    fn showing_and_listing_contexts_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx_in(dir.path());
        // Current one, a named one, an unknown one (an empty list), all.
        ctx.handle_context(None, false, false).unwrap();
        ctx.handle_context(Some("two".to_string()), false, false)
            .unwrap();
        ctx.handle_context(Some("nope".to_string()), false, false)
            .unwrap();
        ctx.handle_context(None, true, false).unwrap();
        assert_eq!(current_on_disk(&ctx).as_deref(), Some("one"));

        // No current context at all: an empty listing, not an error.
        ctx.kubeconfig.current_context = None;
        ctx.handle_context(None, false, false).unwrap();
    }

    #[test]
    fn use_context_writes_the_new_current_context() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx_in(dir.path());
        ctx.handle_context(Some("two".to_string()), false, true)
            .unwrap();
        assert_eq!(current_on_disk(&ctx).as_deref(), Some("two"));
    }

    #[test]
    fn use_context_without_a_name_or_with_an_unknown_one_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx_in(dir.path());
        assert!(matches!(
            ctx.handle_context(None, false, true),
            Err(ProxyAuthK8sError::InvalidUsage(_))
        ));
        assert!(matches!(
            ctx.handle_context(Some("nope".to_string()), false, true),
            Err(ProxyAuthK8sError::InvalidUsage(_))
        ));
        // Nothing was written.
        assert_eq!(current_on_disk(&ctx).as_deref(), Some("one"));
    }

    #[test]
    fn use_context_on_a_kubeconfig_gone_from_disk_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx_in(dir.path());
        // Known in memory, but the file to rewrite cannot be read back.
        ctx.kubeconfig_path = crate::helper::unwritable_path(dir.path(), "kubeconfig");
        assert!(matches!(
            ctx.handle_context(Some("two".to_string()), false, true),
            Err(ProxyAuthK8sError::KubeconfigReadError(_))
        ));
    }
}
