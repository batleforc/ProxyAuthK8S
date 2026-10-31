import type { CallbackModel } from '@proxy-auth-k8s/front-api';

/** Shape of `clustersStore.callBack`: the cluster being logged into and the API answer. */
export interface ClusterCallbackData {
  ns: string;
  cluster: string;
  retour: CallbackModel;
}

interface TokenKubeconfig {
  clusterName: string;
  userName: string;
  contextName: string;
  server: string;
  token: string;
}

function renderTokenKubeconfig({
  clusterName,
  userName,
  contextName,
  server,
  token,
}: TokenKubeconfig) {
  return `apiVersion: v1
kind: Config
clusters:
- name: ${clusterName}
  cluster:
    server: ${server}
    insecure-skip-tls-verify: false
users:
- name: ${userName}
  user:
    token: ${token}
contexts:
- name: ${contextName}
  context:
    cluster: ${clusterName}
    user: ${userName}
current-context: ${contextName}`;
}

/** Kubeconfig embedding the static id token returned by the cluster callback. */
export function generateCallbackKubeconfig(data: ClusterCallbackData): string {
  if (!data.retour?.access_token) return '';

  return renderTokenKubeconfig({
    clusterName: `${data.ns}-${data.cluster}`,
    userName: `${data.retour.subject}@${data.ns}-${data.cluster}`,
    contextName: `${data.ns}-${data.cluster}-context`,
    server: data.retour.cluster_url,
    token: data.retour.id_token,
  });
}

/** Kubeconfig delegating token retrieval to the `kubectl proxyauth` exec plugin. */
export function generateCallbackPluginKubeconfig(
  data: ClusterCallbackData,
): string {
  if (!data.retour?.access_token) return '';

  const clusterName = `${data.ns}-${data.cluster}`;
  const userName = `${data.retour.subject}@${data.ns}-${data.cluster}`;
  const contextName = `${data.ns}-${data.cluster}-context`;
  // create a const of the cluster_url without the path
  const clusterUrl = new URL(data.retour.cluster_url).origin;

  return `apiVersion: v1
kind: Config
clusters:
- name: ${clusterName}
  cluster:
    server: ${data.retour.cluster_url}
    insecure-skip-tls-verify: false
users:
- name: ${userName}
  user:
    exec:
      apiVersion: client.authentication.k8s.io/v1
      args:
        - proxyauth
        - get-token
        - --namespace=${data.ns}
        - --server-url=${clusterUrl}
        - ${data.cluster}
      command: kubectl
      env: null
      interactiveMode: Never
      provideClusterInfo: false
contexts:
- name: ${contextName}
  context:
    cluster: ${clusterName}
    user: ${userName}
current-context: ${contextName}`;
}

/** Shell commands configuring the `kubectl proxyauth` plugin for this cluster. */
export function generateCallbackPluginCommands(
  data: ClusterCallbackData,
  t: (key: string) => string,
): string {
  if (!data.retour?.access_token) return '';

  const clusterUrl = new URL(data.retour.cluster_url).origin;
  const c = (n: number) => t(`cli.data.callback.c${n}`);

  return `${c(1)}
kubectl proxyauth login --server-url "${clusterUrl}" --token "${data.retour.access_token}"

${c(2)}
kubectl proxyauth -n "${data.ns}" login "${data.cluster}"
${c(3)}
kubectl proxyauth -n "${data.ns}" login "${data.cluster}" --token "${data.retour.id_token}"

${c(4)}
kubectl get pods
kubectl get services

${c(5)}
kubectl proxyauth ctx --set "${data.ns}-${data.cluster}-context"`;
}

/**
 * Kubeconfig for a cluster in token exchange (RFC 0010): the exec plugin hands
 * kubectl the user's session to the proxy, no cluster login is needed.
 */
export function generateExchangeKubeconfig(
  cluster: { namespace: string; name: string } | null | undefined,
  clusterUrl: string,
  username: string,
): string {
  if (!cluster || !clusterUrl) return '';

  const clusterName = `${cluster.namespace}-${cluster.name}`;
  const userName = `${username}@${clusterName}`;
  const contextName = `${clusterName}-context`;
  const serverUrl = new URL(clusterUrl).origin;

  return `apiVersion: v1
kind: Config
clusters:
- name: ${clusterName}
  cluster:
    server: ${clusterUrl}
    insecure-skip-tls-verify: false
users:
- name: ${userName}
  user:
    exec:
      apiVersion: client.authentication.k8s.io/v1
      args:
        - proxyauth
        - get-token
        - --namespace=${cluster.namespace}
        - --server-url=${serverUrl}
        - ${cluster.name}
      command: kubectl
      env: null
      interactiveMode: Never
      provideClusterInfo: false
contexts:
- name: ${contextName}
  context:
    cluster: ${clusterName}
    user: ${userName}
current-context: ${contextName}`;
}

/** Kubeconfig for a non-SSO cluster reached through the proxy (token left to fill in). */
export function generateNoSsoKubeconfig(
  cluster: { namespace: string; name: string } | null | undefined,
  clusterUrl: string,
  username: string,
): string {
  if (!cluster) return '';

  return renderTokenKubeconfig({
    clusterName: `${cluster.namespace}-${cluster.name}`,
    userName: `${username}@${cluster.namespace}-${cluster.name}`,
    contextName: `${cluster.namespace}-${cluster.name}-context`,
    server: clusterUrl,
    token: 'YOUR_ACCESS_TOKEN',
  });
}
