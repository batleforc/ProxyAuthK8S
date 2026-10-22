import { OperatorHome } from '@weebo/operator-docs/site';
import { site } from '@/lib/shared';

export default function HomePage() {
  return (
    <OperatorHome
      site={site}
      label="Proxy d'authentification · Kubernetes"
      caption="Votre passerelle sécurisée vers Kubernetes : une connexion SSO, une interface unique pour tous vos clusters, des règles d'accès par cluster et une trace d'audit, sans impersonation."
      actions={[
        { href: '/docs/quick-started', label: 'Démarrer', primary: true },
        { href: '/docs', label: 'Lire la doc' },
        { href: '/docs/ide', label: 'Ouvrir l’IDE à CRD' },
      ]}
      install={`helm upgrade --install proxyauthk8s-crd \\
  oci://ghcr.io/batleforc/proxyauthk8s/chart-crd \\
  --namespace proxyauthk8s --create-namespace
helm upgrade --install proxyauthk8s \\
  oci://ghcr.io/batleforc/proxyauthk8s/chart \\
  --namespace proxyauthk8s --values proxyauthk8s-values.yaml`}
      example={{
        title: 'Exposer un cluster',
        yaml: `apiVersion: weebo.si.rs/v1
kind: ProxyKubeApi
metadata:
  name: local
  namespace: default
spec:
  cert:
    SystemRoots: true
  service:
    KubernetesService:
      name: kubernetes
      namespace: default
      port: 443`,
      }}
      features={[
        {
          title: 'Sécurité renforcée',
          body: "Authentification centralisée, règles par cluster sur les ressources, les verbes, les ports et l'exec, et chaque décision écrite dans la trace d'audit.",
          href: '/docs/security',
          link: 'Sécurité',
        },
        {
          title: 'CLI et plugin kubectl',
          body: 'Le CLI dédié et le plugin kubectl authentifient votre kubectl auprès du proxy : plus de kubeconfig par cluster à jongler.',
          href: '/docs/kubectl-plugin',
          link: 'Plugin kubectl',
        },
        {
          title: 'Interface unifiée',
          body: "Une seule interface pour retrouver tous vos clusters et générer leur kubeconfig, intégrable à d'autres portails.",
          href: '/docs/ui',
          link: 'Interface',
        },
      ]}
    />
  );
}
