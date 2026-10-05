import { describe, expect, it } from 'vitest';
import {
  backendUrlToName,
  buildUsageExamples,
  buildWorkflowExample,
  installationCommands,
  resolveBackendUrl,
} from './cliCommands';

describe('resolveBackendUrl', () => {
  it.each([
    ['https://api.example.com/api', 'https://api.example.com'],
    ['https://api.example.com/api/', 'https://api.example.com'],
    ['https://api.example.com/', 'https://api.example.com'],
    ['https://api.example.com', 'https://api.example.com'],
  ])('normalises %s', (env, expected) => {
    expect(resolveBackendUrl(env, 'https://front.example.com')).toBe(expected);
  });

  it.each([undefined, '', '/api'])('falls back to the front origin for %s', (env) => {
    expect(resolveBackendUrl(env, 'https://front.example.com')).toBe('https://front.example.com');
  });
});

describe('backendUrlToName', () => {
  it('drops the scheme and replaces the first dot and colon', () => {
    expect(backendUrlToName('https://proxy.example.com')).toBe('proxy-example.com');
    expect(backendUrlToName('http://localhost:8080')).toBe('localhost-8080');
  });
});

describe('CLI guide content', () => {
  it('injects the backend URL and name in the configuration examples', () => {
    const examples = buildUsageExamples('https://proxy.example.com', 'proxy-example.com');
    expect(examples.config.examples.map((e) => e.command)).toEqual([
      'kubectl proxyauth config set-def --default-server "proxy-example.com"',
      'kubectl proxyauth login --server-url "https://proxy.example.com"',
      'kubectl proxyauth config set-def --server "proxy-example.com" --namespace "team-production"',
      'kubectl proxyauth config get --list',
    ]);
    expect(Object.keys(examples)).toEqual(['config', 'auth', 'clusters', 'contexts']);
  });

  it('includes the current token in the workflow', () => {
    const workflow = buildWorkflowExample('https://proxy.example.com', 'proxy-example.com', 'TOKEN');
    expect(workflow).toContain('kubectl proxyauth login --server-url "https://proxy.example.com" --token "TOKEN"');
    expect(workflow.startsWith('# 1. Configuration initiale\n')).toBe(true);
  });

  it('ends every installation method with an install or check command', () => {
    expect(installationCommands.krew.commands.at(-1)).toBe('kubectl krew install proxyauth');
    expect(installationCommands.manual.commands.at(-1)).toBe('kubectl proxyauth --help');
    expect(installationCommands.homebrew.commands.at(-1)).toBe('kubectl proxyauth --help');
  });
});
