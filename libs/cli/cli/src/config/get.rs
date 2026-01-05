use crate::{
    cli_config::cli_server_config::CliServerConfig,
    config::{
        get_output::{GetOutput, VecGetOutput},
        ConfigCommands,
    },
    ctx::CliCtx,
};

impl ConfigCommands {
    pub fn handle_get(&self, ctx: &mut CliCtx) {
        if let ConfigCommands::Get {
            server_url,
            namespace,
            list,
        } = self
        {
            let default_server_name = ctx.config.default_server_name.clone();
            let mut outputs: Vec<GetOutput> = Vec::new();

            if *list {
                for (_name, server_config) in ctx.config.servers.iter() {
                    let output = GetOutput::new_from_servers(
                        server_config.clone(),
                        default_server_name.clone(),
                    );
                    outputs.push(output);
                }
            } else if server_url.is_none() && namespace.is_none() {
                if let Some(default_server_config) = ctx.config.servers.get(&default_server_name) {
                    let output = GetOutput::new_from_servers(
                        default_server_config.clone(),
                        default_server_name.clone(),
                    );
                    outputs.push(output);
                }
            } else {
                for (name, server_config) in ctx.config.servers.iter() {
                    if let Some(filter_url) = server_url {
                        let server_name =
                            CliServerConfig::url_to_name_from_string(filter_url.clone());
                        if !name.starts_with(&server_name) {
                            continue;
                        }
                    }
                    if let Some(filter_namespace) = namespace {
                        if &server_config.namespace != filter_namespace {
                            continue;
                        }
                    }
                    let output = GetOutput::new_from_servers(
                        server_config.clone(),
                        default_server_name.clone(),
                    );
                    outputs.push(output);
                }
            }

            let vec_output = VecGetOutput::new(outputs);
            let output_str = vec_output.to_output(ctx.format.clone());
            println!("{}", output_str);
        }
    }
}
