/**
 * Static content and command builders for the `kubectl proxyauth` CLI guide (CliView).
 */

/** A translation function (vue-i18n's `t`): keeps these builders free of Vue. */
export type Translate = (key: string) => string;

export interface InstallationMethod {
  title: string;
  description: string;
  commands: string[];
  badge: string;
}

export interface UsageExample {
  title: string;
  command: string;
  description: string;
}

export interface UsageSection {
  title: string;
  description: string;
  examples: UsageExample[];
}

/**
 * Backend base URL used in the CLI examples: the build-time `VITE_API_BASE_URL`
 * stripped of a trailing `/` or `/api`, or the front origin when unset/empty.
 */
export function resolveBackendUrl(
  envUrl: string | undefined,
  origin: string,
): string {
  if (envUrl && (envUrl.endsWith('/') || envUrl.endsWith('/api'))) {
    envUrl = envUrl.replace(/\/api\/?$/, '').replace(/\/$/, '');
  }
  if (envUrl === '' || envUrl === undefined) {
    // IF the env variable is an empty string, use the frontend url
    envUrl = origin;
  }
  return envUrl;
}

/** Server name the CLI derives from a backend URL (scheme dropped, first `.`/`:` turned into `-`). */
export function backendUrlToName(url: string): string {
  return url
    .replace('http://', '')
    .replace('https://', '')
    .replace('.', '-')
    .replace(':', '-');
}

const D = 'cli.data';

/** Installation commands for each platform, in the current locale. */
export function buildInstallationCommands(t: Translate) {
  const i = (key: string) => t(`${D}.install.${key}`);
  return {
    krew: {
      title: i('krew.title'),
      description: i('krew.description'),
      commands: [
        i('krew.c1'),
        'curl -fsSLO "https://github.com/kubernetes-sigs/krew/releases/latest/download/krew-{linux_amd64,darwin_amd64,windows_amd64}.tar.gz"',
        'tar zxvf krew-*.tar.gz',
        './krew-* install krew',
        '',
        i('krew.c2'),
        'kubectl krew install proxyauth',
      ],
      badge: i('krew.badge'),
    },
    manual: {
      title: i('manual.title'),
      description: i('manual.description'),
      commands: [
        i('manual.c1'),
        'wget https://github.com/batleforc/proxyauthk8s/releases/latest/download/kubectl-proxyauth-linux-amd64',
        '',
        i('manual.c2'),
        'chmod +x kubectl-proxyauth-linux-amd64',
        '',
        i('manual.c3'),
        'sudo mv kubectl-proxyauth-linux-amd64 /usr/local/bin/kubectl-proxyauth',
        '',
        i('manual.c4'),
        'kubectl proxyauth --help',
      ],
      badge: i('manual.badge'),
    },
    homebrew: {
      title: i('homebrew.title'),
      description: i('homebrew.description'),
      commands: [
        i('homebrew.c1'),
        'brew tap batleforc/proxyauthk8s',
        '',
        i('homebrew.c2'),
        'brew install kubectl-proxyauth',
        '',
        i('homebrew.c3'),
        'kubectl proxyauth --help',
      ],
      badge: i('homebrew.badge'),
    },
  } satisfies Record<string, InstallationMethod>;
}

/** Usage examples, in the current locale. */
export function buildUsageExamples(
  backendUrl: string,
  backendUrlName: string,
  t: Translate,
) {
  const section = (key: string, commands: string[]): UsageSection => ({
    title: t(`${D}.usage.${key}.title`),
    description: t(`${D}.usage.${key}.description`),
    examples: commands.map((command, index) => ({
      title: t(`${D}.usage.${key}.e${index + 1}.title`),
      command,
      description: t(`${D}.usage.${key}.e${index + 1}.description`),
    })),
  });
  return {
    config: section('config', [
      `kubectl proxyauth config set-def --default-server "${backendUrlName}"`,
      `kubectl proxyauth login --server-url "${backendUrl}"`,
      `kubectl proxyauth config set-def --server "${backendUrlName}" --namespace "team-production"`,
      'kubectl proxyauth config get --list',
    ]),
    auth: section('auth', [
      `kubectl proxyauth login --server-url <url> --token ${t(`${D}.tokenPlaceholder`)}`,
      'kubectl proxyauth login my-cluster',
      'kubectl proxyauth get-token my-cluster',
      'kubectl proxyauth logout my-cluster',
      'kubectl proxyauth cache clear',
    ]),
    clusters: section('clusters', [
      'kubectl proxyauth get',
      'kubectl proxyauth get my-cluster --format yaml',
      'kubectl proxyauth get --namespace production',
    ]),
    contexts: section('contexts', [
      'kubectl proxyauth ctx --list',
      'kubectl proxyauth ctx --set my-cluster',
      'kubectl proxyauth ctx',
    ]),
  } satisfies Record<string, UsageSection>;
}

export type UsageExamples = ReturnType<typeof buildUsageExamples>;

/** The complete workflow example, in the current locale. */
export function buildWorkflowExample(
  backendUrl: string,
  backendUrlName: string,
  token: string | undefined,
  t: Translate,
): string {
  const c = (n: number) => t(`${D}.workflow.c${n}`);
  return `${c(1)}
kubectl proxyauth config set-def --default-server "${backendUrlName}"
kubectl proxyauth config set-def --server "${backendUrl}" --namespace "default"

${c(2)}
kubectl proxyauth login
${c(3)}
kubectl proxyauth login --server-url "${backendUrl}" --token "${token}"

${c(4)}
kubectl proxyauth get

${c(5)}
kubectl proxyauth login my-production-cluster

${c(6)}
kubectl get pods
kubectl get services

${c(7)}
kubectl proxyauth ctx --set another-cluster
kubectl get nodes`;
}
