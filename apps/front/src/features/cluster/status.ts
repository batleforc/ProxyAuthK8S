import type { VisibleCluster } from '@proxy-auth-k8s/front-api';

/** The four cluster statuses of `ui.mdx`, in the order they are decided. */
export type ClusterStatusKind =
  'disabled' | 'unreachable' | 'available' | 'unknown';

export function clusterStatus(cluster: VisibleCluster): ClusterStatusKind {
  if (!cluster.enabled) return 'disabled';
  if (cluster.is_reachable === false) return 'unreachable';
  if (cluster.is_reachable === true) return 'available';
  return 'unknown';
}

/** The proxy URL a kubeconfig points at for `cluster`. */
export function clusterProxyUrl(
  cluster: Pick<VisibleCluster, 'namespace' | 'name'>,
): string {
  return `${globalThis.location.origin}/clusters/${cluster.namespace}/${cluster.name}`;
}
