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
    mise: {
      title: i('mise.title'),
      description: i('mise.description'),
      commands: [
        i('mise.c1'),
        'curl https://mise.run | sh',
        '',
        i('mise.c2'),
        'mise use -g github:batleforc/ProxyAuthK8S',
      ],
      badge: i('mise.badge'),
    },
    manual: {
      title: i('manual.title'),
      description: i('manual.description'),
      commands: [
        i('manual.c1'),
        'VERSION=$(curl -fsSLI -o /dev/null -w "%{url_effective}" https://github.com/batleforc/ProxyAuthK8S/releases/latest | sed "s|.*/||")',
        'curl -fsSLO "https://github.com/batleforc/ProxyAuthK8S/releases/download/$VERSION/kubectl-proxyauth_${VERSION}_linux_amd64.tar.gz"',
        '',
        i('manual.c2'),
        'tar xzf kubectl-proxyauth_*_linux_amd64.tar.gz',
        '',
        i('manual.c3'),
        'sudo install kubectl-proxyauth /usr/local/bin/',
        '',
        i('manual.c4'),
        'kubectl proxyauth --help',
      ],
      badge: i('manual.badge'),
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
