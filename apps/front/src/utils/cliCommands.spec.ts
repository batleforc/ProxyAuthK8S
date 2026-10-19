import { describe, expect, it } from 'vitest';
import { i18n } from '../i18n';
import {
  backendUrlToName,
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

describe('backendUrlToName', () => {
  it('drops the scheme and replaces the first dot and colon', () => {
    expect(backendUrlToName('https://proxy.example.com')).toBe(
      'proxy-example.com',
    );
    expect(backendUrlToName('http://localhost:8080')).toBe('localhost-8080');
  });
});

describe('CLI guide content', () => {
  it('injects the backend URL and name in the configuration examples', () => {
    const examples = buildUsageExamples(
      'https://proxy.example.com',
      'proxy-example.com',
      fr,
    );
    expect(examples.config.examples.map((e) => e.command)).toEqual([
      'kubectl proxyauth config set-def --default-server "proxy-example.com"',
      'kubectl proxyauth login --server-url "https://proxy.example.com"',
      'kubectl proxyauth config set-def --server "proxy-example.com" --namespace "team-production"',
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
      'proxy-example.com',
      'TOKEN',
      fr,
    );
    expect(workflow).toContain(
      'kubectl proxyauth login --server-url "https://proxy.example.com" --token "TOKEN"',
    );
    expect(workflow.startsWith('# 1. Configuration initiale\n')).toBe(true);
  });

  it('follows the locale, commands unchanged', () => {
    const inEn = buildUsageExamples('https://p.example', 'p-example', en);
    const inFr = buildUsageExamples('https://p.example', 'p-example', fr);
    expect(inEn.auth.title).toBe('Authentication');
    expect(inFr.auth.title).toBe('Authentification');
    expect(inEn.config.examples.map((e) => e.command)).toEqual(
      inFr.config.examples.map((e) => e.command),
    );
    expect(buildInstallationCommands(en).krew.badge).toBe('Recommended');
    expect(
      buildWorkflowExample('https://p.example', 'p-example', 'T', en),
    ).toMatch(/^# 1\. Initial configuration\n/);
    // Every key resolves: none is printed raw.
    const all = JSON.stringify([
      buildUsageExamples('u', 'n', en),
      buildInstallationCommands(en),
      buildWorkflowExample('u', 'n', 'T', en),
    ]);
    expect(all).not.toContain('cli.data');
  });

  it('ends every installation method with an install or check command', () => {
    const installationCommands = buildInstallationCommands(fr);
    expect(installationCommands.krew.commands.at(-1)).toBe(
      'kubectl krew install proxyauth',
    );
    expect(installationCommands.manual.commands.at(-1)).toBe(
      'kubectl proxyauth --help',
    );
    expect(installationCommands.homebrew.commands.at(-1)).toBe(
      'kubectl proxyauth --help',
    );
  });
});
