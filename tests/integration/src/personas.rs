//! The bench's users, from `.it/personas.yaml` (the file Dex is rendered
//! from). A test picks one by name and never defines its own.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::Bench;

/// The dashboard's OIDC client in Dex (`OIDC_CLIENT_ID` of the back).
const DASHBOARD_CLIENT: &str = "proxyauthk8s";

#[derive(Debug, Clone, Deserialize)]
pub struct Persona {
    #[serde(skip)]
    pub name: String,
    pub email: String,
    pub password: String,
    #[serde(default)]
    pub groups: Vec<String>,
}

#[derive(Deserialize)]
struct File {
    personas: BTreeMap<String, Persona>,
}

impl Persona {
    /// The persona called `name`; panics on an unknown one.
    pub fn named(name: &str) -> Persona {
        let path = Bench::get().root.join(".it/personas.yaml");
        let text = std::fs::read_to_string(&path).expect("read .it/personas.yaml");
        let file: File = serde_yaml_ng::from_str(&text).expect("parse .it/personas.yaml");
        let mut persona = file
            .personas
            .get(name)
            .unwrap_or_else(|| panic!("no persona {name:?} in .it/personas.yaml"))
            .clone();
        persona.name = name.to_string();
        persona
    }

    /// A dashboard access token for this persona, straight from Dex (password
    /// grant): the token a user copies from the dashboard for
    /// `kubectl proxyauth login --token`.
    pub async fn dashboard_token(&self) -> String {
        let bench = Bench::get();
        let response = bench
            .http()
            .post(format!("{}/token", bench.dex_url()))
            .form(&[
                ("grant_type", "password"),
                ("client_id", DASHBOARD_CLIENT),
                ("username", self.email.as_str()),
                ("password", self.password.as_str()),
                ("scope", "openid profile email groups"),
            ])
            .send()
            .await
            .expect("Dex token endpoint");
        let status = response.status();
        let body: serde_json::Value = response.json().await.expect("Dex token response");
        assert!(status.is_success(), "Dex refused {}: {body}", self.name);
        body["access_token"]
            .as_str()
            .expect("access_token in Dex's response")
            .to_string()
    }
}
