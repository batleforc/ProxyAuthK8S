import { describe, expect, it } from 'vitest';
import { i18n } from '../i18n';
import {
  buildInstallationCommands,
  buildUsageExamples,
  buildWorkflowExample,
  resolveBackendUrl,
} from './cliCommands';

const tIn = (locale: 'en' | 'fr') => (key: string) =>
  i18n.global.t(key, {}, { locale });
const fr = tIn('fr');
const en = tIn('en');

describe('resolveBackendUrl', () => {
  it.each([
    ['https://api.example.com/api', 'https://api.example.com'],
    ['https://api.example.com/api/', 'https://api.example.com'],
    ['https://api.example.com/', 'https://api.example.com'],
    ['https://api.example.com', 'https://api.example.com'],
  ])('normalises %s', (env, expected) => {
    expect(resolveBackendUrl(env, 'https://front.example.com')).toBe(expected);
  });

  it.each([undefined, '', '/api'])(
    'falls back to the front origin for %s',
    (env) => {
      expect(resolveBackendUrl(env, 'https://front.example.com')).toBe(
        'https://front.example.com',
      );
    },
  );
});

describe('CLI guide content', () => {
  it('injects the backend URL and name in the configuration examples', () => {
    const examples = buildUsageExamples('https://proxy.example.com', fr);
    expect(examples.config.examples.map((e) => e.command)).toEqual([
      'kubectl proxyauth config use-server "https://proxy.example.com"',
      'kubectl proxyauth login --server-url "https://proxy.example.com"',
      'kubectl proxyauth config set-namespace "https://proxy.example.com" "team-production"',
      'kubectl proxyauth config get --list',
    ]);
    expect(Object.keys(examples)).toEqual([
      'config',
      'auth',
      'clusters',
      'contexts',
    ]);
  });

  it('includes the current token in the workflow', () => {
    const workflow = buildWorkflowExample(
      'https://proxy.example.com',
      'TOKEN',
      fr,
    );
    expect(workflow).toContain(
      'kubectl proxyauth login --server-url "https://proxy.example.com" --token "TOKEN"',
    );
    // Login first: `config` only acts on a server already logged in to.
    expect(
      workflow.startsWith(
        '# 1. Se connecter au serveur (il devient le serveur par défaut)\nkubectl proxyauth login --server-url',
      ),
    ).toBe(true);
    expect(workflow).not.toContain('set-def');
    expect(workflow).toContain(
      'kubectl proxyauth use-context default-another-cluster-context',
    );
  });

  it('follows the locale, commands unchanged', () => {
    const inEn = buildUsageExamples('https://p.example', en);
    const inFr = buildUsageExamples('https://p.example', fr);
    expect(inEn.auth.title).toBe('Authentication');
    expect(inFr.auth.title).toBe('Authentification');
    expect(inEn.config.examples.map((e) => e.command)).toEqual(
      inFr.config.examples.map((e) => e.command),
    );
    expect(buildInstallationCommands(en).mise.badge).toBe('Recommended');
    expect(buildWorkflowExample('https://p.example', 'T', en)).toMatch(
      /^# 1\. Log in to the server/,
    );
    // Every key resolves: none is printed raw.
    const all = JSON.stringify([
      buildUsageExamples('u', en),
      buildInstallationCommands(en),
      buildWorkflowExample('u', 'T', en),
    ]);
    expect(all).not.toContain('cli.data');
  });

  it('ends every installation method with an install or check command', () => {
    const installationCommands = buildInstallationCommands(fr);
    expect(installationCommands.mise.commands.at(-1)).toBe(
      'mise use -g github:batleforc/ProxyAuthK8S',
    );
    expect(installationCommands.manual.commands.at(-1)).toBe(
      'kubectl proxyauth --help',
    );
  });
});

describe('CLI commands shown in the UI', () => {
  // Forms the CLI deprecated or never had: the guide must teach the current
  // verbs (docs/content/docs/kubectl-plugin.mdx).
  const STALE = [
    /config set-def/,
    /ctx --(set|list)/,
    /context --(set|list)/,
    /--server "/,
  ];

  it('use no deprecated or unknown command form', () => {
    const examples = buildUsageExamples('https://p.example', fr);
    const commands = [
      ...Object.values(examples).flatMap((s) =>
        s.examples.map((e) => e.command),
      ),
      buildWorkflowExample('https://p.example', 'T', fr),
    ];
    for (const command of commands) {
      for (const stale of STALE) expect(command).not.toMatch(stale);
    }
    expect(examples.auth.examples.map((e) => e.command)).toContain(
      'kubectl proxyauth status',
    );
  });
});
