use crate::{
    cli_config::cli_server_config::CliServerConfig, config_cmd::get_output::GetOutput, ctx::CliCtx,
    output::KubeList,
};

impl CliCtx {
    pub fn handle_get_config(
        &self,
        server_url: Option<&String>,
        namespace: Option<&String>,
        list: bool,
    ) {
        let default_server_name = self.config.default_server_name.clone();
        let mut outputs: Vec<GetOutput> = Vec::new();

        if list {
            for server_config in self.config.servers.values() {
                let output =
                    GetOutput::new_from_servers(server_config.clone(), default_server_name.clone());
                outputs.push(output);
            }
        } else if server_url.is_none() && namespace.is_none() {
            if let Some(default_server_config) = self.config.servers.get(&default_server_name) {
                let output = GetOutput::new_from_servers(
                    default_server_config.clone(),
                    default_server_name.clone(),
                );
                outputs.push(output);
            }
        } else {
            for (name, server_config) in &self.config.servers {
                if let Some(filter_url) = server_url {
                    let server_name = CliServerConfig::url_to_name_from_string(filter_url.clone());
                    if !name.starts_with(&server_name) {
                        continue;
                    }
                }
                if let Some(filter_namespace) = namespace {
                    if &server_config.namespace != filter_namespace {
                        continue;
                    }
                }
                let output =
                    GetOutput::new_from_servers(server_config.clone(), default_server_name.clone());
                outputs.push(output);
            }
        }

        let vec_output = KubeList::new(outputs);
        let output_str = vec_output.to_output(self.format.clone());
        println!("{output_str}");
    }
}
