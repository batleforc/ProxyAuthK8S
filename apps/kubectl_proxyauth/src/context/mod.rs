use tracing::info;

use crate::{
    context::output::{GetContextOutput, VecGetContextOutput},
    ctx::CliCtx,
};

pub mod output;

impl CliCtx {
    pub fn handle_context(&mut self, context_name: Option<String>, list: bool, set: bool) {
        if set && context_name.is_none() {
            panic!("Context name must be provided when using the --set flag.");
        } else if set {
            let context_name = context_name.clone().unwrap();
            // Find the context in the kubeconfig
            let context = self
                .kubeconfig
                .contexts
                .iter()
                .find(|ctx| ctx.name == context_name);
            match context {
                Some(_) => {
                    // Set the current context
                    info!("Setting current context to: {}", context_name);
                    // Here you would implement the logic to actually set the context
                    self.kubeconfig.current_context = Some(context_name);
                }
                None => {
                    panic!("Context '{}' not found in kubeconfig.", context_name);
                }
            };
        }
        let vec_context = if list {
            // Map all contexts from Kubeconfig to GetContextOutput
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
        let output = VecGetContextOutput::new(vec_context);
        println!("{}", output.to_output(self.format.clone()));
    }
}
