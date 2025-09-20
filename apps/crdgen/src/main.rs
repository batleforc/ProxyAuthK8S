use crd::ProxyKubeApi;
use kube::CustomResourceExt;

fn main() {
    print!("{}", serde_yaml::to_string(&ProxyKubeApi::crd()).unwrap());
}
