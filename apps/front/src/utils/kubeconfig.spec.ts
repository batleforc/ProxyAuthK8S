import { describe, expect, it } from 'vitest';
import { i18n } from '../i18n';
import {
  type ClusterCallbackData,
  generateCallbackKubeconfig,
  generateCallbackPluginCommands,
  generateCallbackPluginKubeconfig,
  generateExchangeKubeconfig,
  generateNoSsoKubeconfig,
} from './kubeconfig';

const callback: ClusterCallbackData = {
  ns: 'team',
  cluster: 'prod',
  retour: {
    access_token: 'ACCESS',
    id_token: 'ID',
    refresh_token: 'REFRESH',
    subject: 'jane',
    cluster_url: 'https://proxy.example.com/clusters/team/prod',
  },
};

const noToken: ClusterCallbackData = {
  ...callback,
  retour: { ...callback.retour, access_token: '' },
};

describe('generateCallbackKubeconfig', () => {
  it('renders a kubeconfig embedding the id token', () => {
    expect(generateCallbackKubeconfig(callback)).toBe(`apiVersion: v1
kind: Config
clusters:
- name: team-prod
  cluster:
    server: https://proxy.example.com/clusters/team/prod
    insecure-skip-tls-verify: false
users:
- name: jane@team-prod
  user:
    token: ID
contexts:
- name: team-prod-context
  context:
    cluster: team-prod
    user: jane@team-prod
current-context: team-prod-context`);
  });

  it('is empty without an access token', () => {
    expect(generateCallbackKubeconfig(noToken)).toBe('');
  });
});

describe('generateCallbackPluginKubeconfig', () => {
  it('delegates to the exec plugin with the proxy origin', () => {
    const yaml = generateCallbackPluginKubeconfig(callback);
    expect(yaml).toContain(`  user:
    exec:
      apiVersion: client.authentication.k8s.io/v1
      args:
        - proxyauth
        - get-token
        - --namespace=team
        - --server-url=https://proxy.example.com
        - prod
      command: kubectl`);
    expect(yaml).toContain(
      '    server: https://proxy.example.com/clusters/team/prod\n',
    );
    expect(yaml).not.toContain('token: ID');
    expect(yaml.endsWith('current-context: team-prod-context')).toBe(true);
  });

  it('is empty without an access token', () => {
    expect(generateCallbackPluginKubeconfig(noToken)).toBe('');
  });
});

describe('generateCallbackPluginCommands', () => {
  it('logs in with the access token and the cluster with the id token', () => {
    const commands = generateCallbackPluginCommands(callback, (k) =>
      i18n.global.t(k),
    );
    expect(commands).toContain(
      'kubectl proxyauth login --server-url "https://proxy.example.com" --token "ACCESS"',
    );
    expect(commands).toContain('kubectl proxyauth -n "team" login "prod"\n');
    expect(commands).toContain(
      'kubectl proxyauth -n "team" login "prod" --token "ID"',
    );
    expect(
      commands.endsWith('kubectl proxyauth ctx --set "team-prod-context"'),
    ).toBe(true);
  });

  it('is empty without an access token', () => {
    expect(
      generateCallbackPluginCommands(noToken, (k) => i18n.global.t(k)),
    ).toBe('');
  });
});

describe('generateNoSsoKubeconfig', () => {
  it('points at the proxy URL with a placeholder token', () => {
    const yaml = generateNoSsoKubeconfig(
      { namespace: 'team', name: 'legacy' },
      'https://proxy.example.com/clusters/team/legacy',
      'jane',
    );
    expect(yaml).toContain(
      '    server: https://proxy.example.com/clusters/team/legacy\n',
    );
    expect(yaml).toContain(
      '- name: jane@team-legacy\n  user:\n    token: YOUR_ACCESS_TOKEN\n',
    );
    expect(yaml.endsWith('current-context: team-legacy-context')).toBe(true);
  });

  it('is empty without a cluster', () => {
    expect(generateNoSsoKubeconfig(null, '', 'jane')).toBe('');
  });
});

describe('generateExchangeKubeconfig', () => {
  const cluster = { namespace: 'team', name: 'prod' };

  it('asks the exec plugin for the proxy session, never embeds a token', () => {
    const yaml = generateExchangeKubeconfig(
      cluster,
      'https://proxy.example.com/clusters/team/prod',
      'alice',
    );
    expect(yaml).toContain(`        - get-token
        - --namespace=team
        - --server-url=https://proxy.example.com
        - prod`);
    expect(yaml).toContain(
      '    server: https://proxy.example.com/clusters/team/prod\n',
    );
    expect(yaml).not.toContain('token:');
    expect(yaml.endsWith('current-context: team-prod-context')).toBe(true);
  });

  it('is empty without a cluster or its URL', () => {
    expect(generateExchangeKubeconfig(null, 'https://p', 'alice')).toBe('');
    expect(generateExchangeKubeconfig(cluster, '', 'alice')).toBe('');
  });
});
