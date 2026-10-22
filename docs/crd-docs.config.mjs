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
  // The CRD IDE's references, on top of crdgen's links.json (the objects the
  // fields make, the schematic kinds): the crd types carry no x-weebo-link.
  links: {
    ProxyKubeApi: {
      references: [
        ...[
          'spec.auth_config.oidc_provider.config_from.Secret',
          'spec.cert.Secret',
          'spec.client_cert.cert.Secret',
          'spec.client_cert.key.Secret',
          'spec.virtual_apis[].list_fallback_token.Secret',
        ].map((path) => ({ path, to: [{ kind: 'Secret' }] })),
        { path: 'spec.service.KubernetesService', to: [{ kind: 'Service' }] },
      ],
    },
  },
});
