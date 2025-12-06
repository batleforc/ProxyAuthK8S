# Kubectl ProxyAuth

A `Kubectl` plugin to ease the burden of authenticating against multiple Kubernetes Cluster Apis through the ProxyAuthK8s service.

This document is only meant to describe the functionality of the plugin that needs to be developed.

## Overview

The `kubectl-proxyauth` plugin will allow users to authenticate against multiple Kubernetes clusters using the ProxyAuthK8s service. The plugin will handle the authentication process, making it easier for users to switch between different clusters without having to manually manage authentication tokens or credentials.

## Features

- **Default Parameters**: The plugin will use default parameters for the ProxyAuthK8s service, which can be overridden by user-provided configurations or environment variables.
  - `--namespace (-n) <namespace>` : Optionally specify a namespace to filter clusters, if none see only default ns one, if none readable in ns, send a no resource allowed in this ns.
  - `--kubeconfig (-k) <path>` : Optionally specify a kubeconfig file to use for authentication.
  - `--verbose (-v)` : Enable verbose logging for debugging purposes.
  - `--server-url (-s) <url>` : Optionally specify the ProxyAuthK8s service URL, if not provided will use the value from the config file or default to `http://localhost:8080`.
  - `--format (-f) <format>` : Specify the output format (e.g., json, yaml, table). Default is `table`.
- **Cluster Management**:
  - `get`
    - `<cluster-name>` : Retrieves details of the specified cluster.
    - If no cluster name is provided, lists all available clusters.
- **Authentication**:
  - `login`
    - `<cluster-name>` : If provided, logs into the specified cluster, if already logged in, will try return the token if still valid or refresh it.
    - If no cluster name is provided, will login to the application
    - `--token (-t) <token>` : Optionally provide a token for authentication, if not provided, will prompt the user to enter one.
  - `logout`
    - `<cluster-name>` : If provided, logs out from the specified cluster.
    - If no cluster name is provided, will logout from the application
  - `cache`
    - `clear` : Clears the cached authentication tokens for all clusters.
  - `get-token`
    - `<cluster-name>` : Retrieves the current authentication token for the specified cluster.
- **Context Handling**: The plugin will manage Kubernetes contexts to ensure that users are authenticated against the correct cluster.
  - `ctx`
    - `<cluster-name>` : Switches the current context to the specified cluster.
    - Output the current context if no cluster name is provided.
    - `list` : Lists all available contexts, and which one come from ProxyAuthK8s or are active.
- **Help Command**: A `--help` flag will be available to provide users with information about the plugin's commands and usage.
- **Error Handling**: The plugin will handle errors gracefully, providing meaningful messages to the user in case of authentication failures or other issues.
  - Each error needs to have a unique ID for easier troubleshooting
  - Each error should be in the linked in the docs error section.
- **Configuration**:
  - Works with existing kubeconfig files or targeted ones.
  - The token cache will be handled like [Kubelogin](https://github.com/int128/kubelogin/blob/master/docs/usage.md#token-cache) has much as possible. [Rust keyring](https://crates.io/crates/keyring) can be used to store tokens in the keyring like kubelogin.
  - Another config file will be used to store where the ProxyAuthK8s service is located, and other plugin specific settings.

### Example Kubeconfig Exec Section

```yaml
  - name: admin@talos-default
    user:
      exec:
        apiVersion: client.authentication.k8s.io/v1beta1
        args:
          - proxyauth
          - get-token
          - -n default
          - -s https://proxyauthk8s.k8s.localhost
          - local-sso
        command: kubectl
        env: null
        provideClusterInfo: false
```

## Useful Links

- [Kubectl Plugin Development Guide](https://kubernetes.io/docs/tasks/extend-kubectl/kubectl-plugins/)
- [Krew example manifest in rust - view allocations](https://github.com/kubernetes-sigs/krew-index/blob/master/plugins/view-allocations.yaml)
- [Krew - Developer Guide](https://krew.sigs.k8s.io/docs/developer-guide/develop/plugin-development/)
- [Kubeconfig - Exec Config](https://kubernetes.io/docs/reference/config-api/kubeconfig.v1/#ExecConfig)
- [Kubectl - Exec Credential Spec v1](https://kubernetes.io/docs/reference/config-api/client-authentication.v1/#client-authentication-k8s-io-v1-ExecCredentialSpec)
- [Kubelogin - Best oidc plugin](https://github.com/int128/kubelogin)
- [Kubelogin - Token Cache](https://github.com/int128/kubelogin/blob/master/docs/usage.md#token-cache)
- [Prettytable-rs - Table formatting](https://github.com/phsym/prettytable-rs)
- [KV-RS - Key Value storage](https://docs.rs/kv/latest/kv/)
