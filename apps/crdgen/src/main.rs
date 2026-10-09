use crd::ProxyKubeApi;
use kube::CustomResourceExt;

fn main() {
    print!(
        "{}",
        serde_yaml_ng::to_string(&ProxyKubeApi::crd()).unwrap()
    );
}
