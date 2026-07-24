# Template for the krew-index plugin manifest.
#
# The release workflow substitutes {{VERSION}} and the per-platform
# {{SHA256_*}} checksums, then attaches the result to the GitHub release. To
# publish, open a PR against kubernetes-sigs/krew-index with the rendered file
# under plugins/proxyauth.yaml.
apiVersion: krew.googlecontainertools.github.com/v1alpha2
kind: Plugin
metadata:
  name: proxyauth
spec:
  version: {{VERSION}}
  homepage: https://github.com/batleforc/proxyauthK8s
  shortDescription: Authenticate to Kubernetes clusters through ProxyAuthK8S
  description: |
    ProxyAuthK8S proxies the Kubernetes API of several clusters behind a single
    OIDC-authenticated endpoint. This plugin is the client side: it performs the
    OIDC login, stores the resulting credentials, and acts as a kubectl
    credential plugin so `kubectl` can talk to any exposed cluster.

    Usage:
      kubectl proxyauth login --server https://proxyauthk8s.example.com
      kubectl proxyauth get clusters
      kubectl proxyauth login cluster <namespace>/<cluster>
  caveats: |
    The plugin stores credentials in the operating system keyring. On Linux this
    requires a running Secret Service implementation (gnome-keyring, KWallet).
  platforms:
    - selector:
        matchLabels:
          os: linux
          arch: amd64
      uri: https://github.com/batleforc/proxyauthK8s/releases/download/{{VERSION}}/kubectl-proxyauth_{{VERSION}}_linux_amd64.tar.gz
      sha256: "{{SHA256_LINUX_AMD64}}"
      bin: kubectl-proxyauth
      files:
        - from: kubectl-proxyauth
          to: .
        - from: LICENSE
          to: .
    - selector:
        matchLabels:
          os: darwin
          arch: amd64
      uri: https://github.com/batleforc/proxyauthK8s/releases/download/{{VERSION}}/kubectl-proxyauth_{{VERSION}}_darwin_amd64.tar.gz
      sha256: "{{SHA256_DARWIN_AMD64}}"
      bin: kubectl-proxyauth
      files:
        - from: kubectl-proxyauth
          to: .
        - from: LICENSE
          to: .
    - selector:
        matchLabels:
          os: darwin
          arch: arm64
      uri: https://github.com/batleforc/proxyauthK8s/releases/download/{{VERSION}}/kubectl-proxyauth_{{VERSION}}_darwin_arm64.tar.gz
      sha256: "{{SHA256_DARWIN_ARM64}}"
      bin: kubectl-proxyauth
      files:
        - from: kubectl-proxyauth
          to: .
        - from: LICENSE
          to: .
    - selector:
        matchLabels:
          os: windows
          arch: amd64
      uri: https://github.com/batleforc/proxyauthK8s/releases/download/{{VERSION}}/kubectl-proxyauth_{{VERSION}}_windows_amd64.tar.gz
      sha256: "{{SHA256_WINDOWS_AMD64}}"
      bin: kubectl-proxyauth.exe
      files:
        - from: kubectl-proxyauth.exe
          to: .
        - from: LICENSE
          to: .
