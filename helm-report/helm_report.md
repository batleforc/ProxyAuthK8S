## ⎈ Helm Chart Build & OCI Package Report

### 📈 Version Updates
✅ Updated chart versions to v0.1.0

### 📦 Main Chart (proxyauthk8s)
✅ Main chart linting passed
✅ Main chart templating successful
✅ Main chart packaged: chart-0.1.0.tgz
🚀 Main chart pushed as OCI artifact: oci://ghcr.io/batleforc/proxyauthk8s/helm-proxyauthk8s:0.1.0

### 📦 CRD Chart (proxyauthk8s-crd)
✅ CRD chart linting passed
✅ CRD chart templating successful
✅ CRD chart packaged: chart-crd-0.1.0.tgz
🚀 CRD chart pushed as OCI artifact: oci://ghcr.io/batleforc/proxyauthk8s/helm-proxyauthk8s-crd:0.1.0

### 📋 Chart Information

#### Main Chart Details
```yaml
apiVersion: v2
appVersion: 0.1.0
description: A chart to govern them all
icon: https://raw.githubusercontent.com/batleforc/ProxyAuthK8S/refs/heads/main/art.png
name: chart
type: application
version: 0.1.0

```

#### CRD Chart Details
```yaml
apiVersion: v2
appVersion: 0.1.0
description: A Helm chart for Kubernetes
icon: https://raw.githubusercontent.com/batleforc/ProxyAuthK8S/refs/heads/main/art.png
name: chart-crd
type: application
version: 0.1.0

```

### 🐳 OCI Artifact Information

#### Installation Commands
```bash
# Install CRD chart first
helm install proxyauthk8s-crd oci://ghcr.io/batleforc/proxyauthk8s/helm-proxyauthk8s-crd:0.1.0

# Install main chart
helm install proxyauthk8s oci://ghcr.io/batleforc/proxyauthk8s/helm-proxyauthk8s:0.1.0
```

#### Upgrade Commands
```bash
# Upgrade CRD chart
helm upgrade proxyauthk8s-crd oci://ghcr.io/batleforc/proxyauthk8s/helm-proxyauthk8s-crd:0.1.0

# Upgrade main chart
helm upgrade proxyauthk8s oci://ghcr.io/batleforc/proxyauthk8s/helm-proxyauthk8s:0.1.0
```

### 📦 Local Packaged Charts

- 📦 `chart-0.1.0.tgz` (4.0K)
- 📦 `chart-crd-0.1.0.tgz` (4.0K)

### 🌐 Published OCI Artifacts

- 🎯 **Main Chart**: `oci://ghcr.io/batleforc/proxyauthk8s/helm-proxyauthk8s:0.1.0`
- 🎯 **CRD Chart**: `oci://ghcr.io/batleforc/proxyauthk8s/helm-proxyauthk8s-crd:0.1.0`
