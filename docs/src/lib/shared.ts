import { defineSite } from '@weebo/operator-docs/site';

// What makes this site ProxyAuthK8S's; everything else comes from
// @weebo/operator-docs.
export const site = defineSite({
  name: 'ProxyAuthK8S',
  description: 'An authenticating proxy in front of your Kubernetes API servers: one SSO login, per-cluster rules, an audit trail.',
  git: {
    user: 'batleforc',
    repo: 'ProxyAuthK8S',
    branch: 'main',
    contentDir: 'docs/content/docs',
  },
});
