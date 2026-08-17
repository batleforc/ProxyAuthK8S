use tracing::{error, info};

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
        if set && context_name.is_none() {
            error!("Context name must be provided when using the --set flag.");
            return Err(ProxyAuthK8sError::InvalidUsage(
                "context name must be provided when using --set".to_string(),
            ));
        } else if set {
            let context_name = context_name.clone().unwrap();
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
                self.kubeconfig.current_context = Some(context_name);
                if let Err(e) = self.write_kubeconfig() {
                    error!("Failed to write kubeconfig: {}", e);
                    return Err(e);
                }
            } else {
                error!("Context '{}' not found in kubeconfig.", context_name);
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
