// Input of `weebo-crd-docs` (@weebo/operator-docs), run by `task docs:crds`:
// what the CRD reference pages say beyond the CRDs themselves. Paths default
// to the expected tree (../deploy/crd → content/docs/crds +
// public/crd-schemas).
import { defineConfig } from '@weebo/operator-docs/crd-docs';

export default defineConfig({
  guides: { ProxyKubeApi: ['Exposer un cluster', '/docs/expose-cluster'] },
  // Checked against the schema: a renamed field fails generation.
  examples: {
    ProxyKubeApi: {
      cert: { SystemRoots: true },
      service: { KubernetesService: { name: 'kubernetes', namespace: 'default', port: 443 } },
    },
  },
});
