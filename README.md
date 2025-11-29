# ProxyAuthK8s

![ProxyAuthK8s Logo](art.png)

ProxyAuthK8S is a part of the Weebo Si project, this project is focused on exposing Kube api server with focus on security and ease of use.

In addition to the secured exposure, this project also eases the sharing of kubeconfig files between users and teams.

The base specification of the [project can be found here](https://batleforc.github.io/weebo-si/0.projects/reverse-api-kube-oidc-based.html).

## Useful links

- [Controller-rs](https://github.com/kube-rs/controller-rs/blob/main/src/controller.rs)
- [Monodon nx](https://github.com/cammisuli/monodon/tree/main/packages/rust)
- [dispatch](https://actix.rs/docs/url-dispatch/)

## API Endpoints

[More info about the api here](libs/api/readme.md)

## Frontend

- [UI LIB](https://maz-ui.com/guide/vue)

## Left to do

### v0.1.0

- [ ] Add Oidc token validation

### v0.2.0

- [ ] Create Krew plugin for easier usage
  - <https://github.com/davidB/kubectl-view-allocations/tree/master>
  - <https://github.com/kubernetes-sigs/krew-index/blob/master/plugins/view-allocations.yaml>
  - name: `proxyauthk8s`
  - <https://docs.rs/clap/latest/clap/>
- [ ] Add more tests
- [ ] Add documentation
- [ ] Add security features (details later)
