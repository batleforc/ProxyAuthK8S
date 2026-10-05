# ProxyAuthK8s {{VERSION}} - {{RELEASE_NAME}}

**Release Date:** {{RELEASE_DATE}}

## Overview

PLACE HOLDER FOR A BRIEF DESCRIPTION OF THE RELEASE, HIGHLIGHTING KEY FEATURES, IMPROVEMENTS, AND FIXES INCLUDED IN THIS VERSION.

---

## 📦 Installation

### Prerequisites

- Kubernetes 1.30+
- Helm 3.8+ (OCI registries)
- An OIDC provider
- Redis (single node or cluster)

See the [Quick Started guide](https://batleforc.github.io/ProxyAuthK8S/docs/quick-started) and the
[chart values reference](https://batleforc.github.io/ProxyAuthK8S/docs/helm-chart).

### Using Helm (Recommended)

```bash
# CRDs first
helm upgrade --install proxyauthk8s-crd oci://ghcr.io/batleforc/proxyauthk8s/chart-crd \
  --version {{CHART_VERSION}} \
  --namespace proxyauthk8s --create-namespace

# Then the application
helm upgrade --install proxyauthk8s oci://ghcr.io/batleforc/proxyauthk8s/chart \
  --version {{CHART_VERSION}} \
  --namespace proxyauthk8s \
  --values values.yaml
```

The chart ships a `values.schema.json`: Helm rejects unknown or mistyped values.

### Container images

Multi-arch (`linux/amd64`, `linux/arm64`), signed with cosign (keyless) and carrying
provenance and SBOM attestations:

```bash
docker pull ghcr.io/batleforc/proxyauthk8s/server:{{VERSION}}
docker pull ghcr.io/batleforc/proxyauthk8s/front:{{VERSION}}

# Verify a signature (images and charts)
cosign verify ghcr.io/batleforc/proxyauthk8s/server:{{VERSION}} \
  --certificate-identity-regexp 'https://github.com/batleforc/.*' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
cosign verify ghcr.io/batleforc/proxyauthk8s/chart:{{CHART_VERSION}} \
  --certificate-identity-regexp 'https://github.com/batleforc/.*' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
```

The CycloneDX SBOMs (`server-sbom.json`, `front-sbom.json`) are attached to this release.

### kubectl plugin

With [krew](https://krew.sigs.k8s.io/): use the `proxyauth.yaml` manifest attached to this release
(`kubectl krew install --manifest=proxyauth.yaml`), or download the archive for your platform:

```bash
# linux_amd64, darwin_amd64, darwin_arm64 or windows_amd64
curl -LO https://github.com/batleforc/ProxyAuthK8S/releases/download/{{VERSION}}/kubectl-proxyauth_{{VERSION}}_linux_amd64.tar.gz
tar -xzf kubectl-proxyauth_{{VERSION}}_linux_amd64.tar.gz
sudo install kubectl-proxyauth /usr/local/bin/

kubectl proxyauth --help
```

Each archive has a `.sha256` next to it.

---

## 🔄 Update Instructions

```bash
helm upgrade proxyauthk8s-crd oci://ghcr.io/batleforc/proxyauthk8s/chart-crd \
  --version {{CHART_VERSION}} --namespace proxyauthk8s
helm upgrade proxyauthk8s oci://ghcr.io/batleforc/proxyauthk8s/chart \
  --version {{CHART_VERSION}} --namespace proxyauthk8s --values values.yaml
```

For the plugin: `kubectl krew upgrade proxyauth`, or replace the binary as above.

## 🔗 Related Resources

- [Full Changelog](CHANGELOG.md)
- [Security Policy](SECURITY.md)
- [Documentation](https://batleforc.github.io/ProxyAuthK8S/docs)

---

## 📞 Support & Feedback

- Report bugs: [GitHub Issues](https://github.com/batleforc/ProxyAuthK8S/issues)
- Security concerns: [Security Vulnerability Report](https://github.com/batleforc/ProxyAuthK8S/issues/new?template=security_vulnerability_report.md)
- Discussions: [GitHub Discussions](https://github.com/batleforc/ProxyAuthK8S/discussions)

---

## 🙏 Contributors

Thank you to all contributors who made this release possible:

{{CONTRIBUTORS_LIST}}
