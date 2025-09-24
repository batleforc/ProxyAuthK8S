use kube::Client;

#[derive(Clone)]
pub struct State {
    pub client: Client,
}

impl State {
    pub async fn new() -> Self {
        let client = Client::try_default()
            .await
            .expect("failed to create kube Client");
        Self { client }
    }
}
