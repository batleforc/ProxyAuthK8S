use crate::{
    cli_config::cli_server_config::CliServerConfig, config_cmd::get_output::GetOutput, ctx::CliCtx,
    error::ProxyAuthK8sError, output::KubeList,
};

impl CliCtx {
    pub fn handle_get_config(
        &self,
        server_url: Option<&String>,
        namespace: Option<&String>,
        list: bool,
    ) -> Result<(), ProxyAuthK8sError> {
        let output = KubeList::new(self.config_rows(server_url, namespace, list));
        println!("{}", output.to_output(self.format.clone()));
        Ok(())
    }

    /// The servers `config get` shows: the default one, every one (`list`),
    /// or those matching the URL and namespace filters.
    fn config_rows(
        &self,
        server_url: Option<&String>,
        namespace: Option<&String>,
        list: bool,
    ) -> Vec<GetOutput> {
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
                if let Some(filter_namespace) = namespace
                    && &server_config.namespace != filter_namespace
                {
                    continue;
                }
                let output =
                    GetOutput::new_from_servers(server_config.clone(), default_server_name.clone());
                outputs.push(output);
            }
        }
        outputs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two servers: `a` (default, namespace `team-a`) and `b` (`team-b`).
    fn ctx() -> CliCtx {
        let mut ctx = CliCtx::for_test();
        for (url, ns) in [
            ("https://a.example", "team-a"),
            ("https://b.example", "team-b"),
        ] {
            let name = CliServerConfig::url_to_name_from_string(url.to_string());
            ctx.config
                .get_or_insert_server_config(name, url.to_string())
                .namespace = ns.to_string();
        }
        ctx.config.default_server_name = "a-example".to_string();
        ctx
    }

    fn urls(rows: Vec<GetOutput>) -> Vec<String> {
        let mut urls: Vec<String> = rows.into_iter().map(|row| row.url).collect();
        urls.sort();
        urls
    }

    #[test]
    fn config_get_shows_the_default_server_or_every_one() {
        let ctx = ctx();
        let rows = ctx.config_rows(None, None, false);
        assert_eq!(urls(rows.clone()), ["https://a.example"]);
        assert!(rows[0].is_default);
        assert_eq!(
            urls(ctx.config_rows(None, None, true)),
            ["https://a.example", "https://b.example"]
        );
        ctx.handle_get_config(None, None, true).unwrap();
    }

    #[test]
    fn config_get_filters_by_url_and_namespace() {
        let ctx = ctx();
        let b = "https://b.example".to_string();
        let team_a = "team-a".to_string();
        let team_z = "team-z".to_string();
        assert_eq!(
            urls(ctx.config_rows(Some(&b), None, false)),
            std::slice::from_ref(&b)
        );
        assert_eq!(
            urls(ctx.config_rows(None, Some(&team_a), false)),
            ["https://a.example"]
        );
        // Both filters must match.
        assert!(ctx.config_rows(Some(&b), Some(&team_a), false).is_empty());
        assert!(ctx.config_rows(None, Some(&team_z), false).is_empty());
    }

    #[test]
    fn config_get_without_a_default_server_shows_nothing() {
        let mut ctx = ctx();
        ctx.config.default_server_name = String::new();
        assert!(ctx.config_rows(None, None, false).is_empty());
        ctx.handle_get_config(None, None, false).unwrap();
    }
}
