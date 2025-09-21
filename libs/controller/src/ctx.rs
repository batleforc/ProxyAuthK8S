use kube::Client;

#[derive(Clone)]
pub struct Context {
    /// Kubernetes client
    pub client: Client,
}
