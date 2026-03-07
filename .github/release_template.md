# ProxyAuthK8s Release v{{VERSION}} - {{RELEASE_NAME}}

**Release Date:** {{RELEASE_DATE}}

## Overview

PLACE HOLDER FOR A BRIEF DESCRIPTION OF THE RELEASE, HIGHLIGHTING KEY FEATURES, IMPROVEMENTS, AND FIXES INCLUDED IN THIS VERSION.

---

## 📦 Installation

### Prerequisites

- Kubernetes 1.30+
- Helm 3.0+
- OIDC Provider (optional, if using OIDC authentication)

### Using Helm (Recommended)

```bash
# Install ProxyAuthK8s
helm install proxyauthk8s oci://ghcr.io/batleforc/proxyauthk8s:v{{VERSION}} \
  --namespace proxyauthk8s \
  --create-namespace \
  --values values.yaml
```

### Docker Images

```bash
# Backend/Controller
docker pull ghcr.io/batleforc/proxyauthk8s/server:v{{VERSION}}

# Frontend
docker pull ghcr.io/batleforc/proxyauthk8s/front:v{{VERSION}}

```

### CLI Tool Installation - Not available YET

```bash
# Download the binary
wget https://github.com/batleforc/ProxyAuthK8S/releases/download/v{{VERSION}}/proxyauthk8s-cli-{{OS}}-{{ARCH}}
chmod +x proxyauthk8s-cli-{{OS}}-{{ARCH}}
sudo mv proxyauthk8s-cli-{{OS}}-{{ARCH}} /usr/local/bin/proxyauthk8s

# Verify installation
proxyauthk8s --version
```

---

## 🔄 Update Instructions

### Helm Update

```bash


# Upgrade the release
helm upgrade proxyauthk8s oci://ghcr.io/batleforc/proxyauthk8s:v{{VERSION}} \
  --namespace proxyauthk8s \
  --values values.yaml
```

#### CLI Tool Update - Not available YET

```bash
# Download and replace the binary
wget https://github.com/batleforc/ProxyAuthK8S/releases/download/v{{VERSION}}/proxyauthk8s-cli-{{OS}}-{{ARCH}}
chmod +x proxyauthk8s-cli-{{OS}}-{{ARCH}}
sudo mv proxyauthk8s-cli-{{OS}}-{{ARCH}} /usr/local/bin/proxyauthk8s
```

## 🔗 Related Resources

- [Full Changelog](CHANGELOG.md)
- [Security Policy](SECURITY.md)
- [Installation Guide](https://batleforc.github.io/ProxyAuthK8S/) - WIP
- [Documentation](https://batleforc.github.io/ProxyAuthK8S/docs) - WIP-

---

## 📞 Support & Feedback

- Report bugs: [GitHub Issues](https://github.com/batleforc/ProxyAuthK8S/issues)
- Security concerns: [Security Vulnerability Report](https://github.com/batleforc/ProxyAuthK8S/issues/new?template=security_vulnerability_report.md)
- Discussions: [GitHub Discussions](https://github.com/batleforc/ProxyAuthK8S/discussions)

---

## 🙏 Contributors

Thank you to all contributors who made this release possible:

{{CONTRIBUTORS_LIST}}
