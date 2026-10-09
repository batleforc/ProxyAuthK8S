/**
 * Static content and command builders for the `kubectl proxyauth` CLI guide (CliView).
 */

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
export function resolveBackendUrl(envUrl: string | undefined, origin: string): string {
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
  return url.replace('http://', '').replace('https://', '').replace('.', '-').replace(':', '-');
}

// Installation commands for different platforms
export const installationCommands = {
  krew: {
    title: 'Krew (Recommandé)',
    description: 'Installation via le gestionnaire de plugins Kubernetes officiel',
    commands: [
      '# Installer Krew si pas déjà fait',
      'curl -fsSLO "https://github.com/kubernetes-sigs/krew/releases/latest/download/krew-{linux_amd64,darwin_amd64,windows_amd64}.tar.gz"',
      'tar zxvf krew-*.tar.gz',
      './krew-* install krew',
      '',
      '# Installer le plugin proxyauth',
      'kubectl krew install proxyauth'
    ],
    badge: 'Recommandé'
  },
  manual: {
    title: 'Installation Manuelle',
    description: 'Téléchargement direct du binaire',
    commands: [
      '# Télécharger le binaire pour votre OS',
      'wget https://github.com/batleforc/proxyauthk8s/releases/latest/download/kubectl-proxyauth-linux-amd64',
      '',
      '# Rendre le fichier exécutable',
      'chmod +x kubectl-proxyauth-linux-amd64',
      '',
      '# Déplacer vers un dossier dans PATH',
      'sudo mv kubectl-proxyauth-linux-amd64 /usr/local/bin/kubectl-proxyauth',
      '',
      '# Vérifier l\'installation',
      'kubectl proxyauth --help'
    ],
    badge: 'Manuel'
  },
  homebrew: {
    title: 'Homebrew (macOS)',
    description: 'Installation via Homebrew sur macOS',
    commands: [
      '# Ajouter le tap',
      'brew tap batleforc/proxyauthk8s',
      '',
      '# Installer le plugin',
      'brew install kubectl-proxyauth',
      '',
      '# Vérifier l\'installation',
      'kubectl proxyauth --help'
    ],
    badge: 'macOS'
  }
} satisfies Record<string, InstallationMethod>;

// Usage examples
export function buildUsageExamples(backendUrl: string, backendUrlName: string) {
  return {
    config: {
      title: 'Configuration',
      description: 'Configurer le plugin pour votre environnement',
      examples: [
        {
          title: 'Définir le serveur par défaut',
          command: `kubectl proxyauth config set-def --default-server "${backendUrlName}"`,
          description: 'Configure le serveur ProxyAuthK8s par défaut'
        },
        {
          title: 'Ajouter un nouveau serveur',
          command: `kubectl proxyauth login --server-url "${backendUrl}"`,
          description: 'Ajoute un nouveau serveur avec son URL'
        },
        {
          title: 'Configurer le namespace par défaut',
          command: `kubectl proxyauth config set-def --server "${backendUrlName}" --namespace "team-production"`,
          description: 'Configure le namespace par défaut pour un serveur'
        },
        {
          title: 'Voir la configuration',
          command: 'kubectl proxyauth config get --list',
          description: 'Affiche toute la configuration actuelle'
        }
      ]
    },
    auth: {
      title: 'Authentification',
      description: 'Se connecter et gérer les tokens',
      examples: [
        {
          title: 'Se connecter à l\'application',
          command: 'kubectl proxyauth login --server-url <url> --token <jeton>',
          description: 'Authentification au serveur avec un jeton (demandé si absent)'
        },
        {
          title: 'Se connecter à un cluster spécifique',
          command: 'kubectl proxyauth login my-cluster',
          description: 'Récupère le token pour un cluster particulier'
        },
        {
          title: 'Obtenir un token existant',
          command: 'kubectl proxyauth get-token my-cluster',
          description: 'Récupère le token stocké pour un cluster'
        },
        {
          title: 'Se déconnecter',
          command: 'kubectl proxyauth logout my-cluster',
          description: 'Supprime le token pour un cluster'
        },
        {
          title: 'Vider le cache des tokens',
          command: 'kubectl proxyauth cache clear',
          description: 'Supprime tous les tokens en cache'
        }
      ]
    },
    clusters: {
      title: 'Gestion des Clusters',
      description: 'Lister et gérer les clusters disponibles',
      examples: [
        {
          title: 'Lister tous les clusters',
          command: 'kubectl proxyauth get',
          description: 'Affiche tous les clusters disponibles'
        },
        {
          title: 'Obtenir les détails d\'un cluster',
          command: 'kubectl proxyauth get my-cluster --format yaml',
          description: 'Affiche les informations détaillées d\'un cluster'
        },
        {
          title: 'Filtrer par namespace',
          command: 'kubectl proxyauth get --namespace production',
          description: 'Liste seulement les clusters du namespace spécifié'
        }
      ]
    },
    contexts: {
      title: 'Gestion des Contextes',
      description: 'Gérer les contextes kubectl avec le plugin',
      examples: [
        {
          title: 'Lister les contextes',
          command: 'kubectl proxyauth ctx --list',
          description: 'Affiche tous les contextes et indique ceux gérés par ProxyAuth'
        },
        {
          title: 'Changer de contexte',
          command: 'kubectl proxyauth ctx --set my-cluster',
          description: 'Définit le contexte actuel'
        },
        {
          title: 'Voir le contexte actuel',
          command: 'kubectl proxyauth ctx',
          description: 'Affiche le contexte actuellement actif'
        }
      ]
    }
  } satisfies Record<string, UsageSection>;
}

export type UsageExamples = ReturnType<typeof buildUsageExamples>;

// Complete workflow example
export function buildWorkflowExample(
  backendUrl: string,
  backendUrlName: string,
  token: string | undefined,
): string {
  return `# 1. Configuration initiale
kubectl proxyauth config set-def --default-server "${backendUrlName}"
kubectl proxyauth config set-def --server "${backendUrl}" --namespace "default"

# 2. Authentification
kubectl proxyauth login
# Or with your current token
kubectl proxyauth login --server-url "${backendUrl}" --token "${token}"

# 3. Lister les clusters disponibles
kubectl proxyauth get

# 4. Se connecter à un cluster spécifique
kubectl proxyauth login my-production-cluster

# 5. Utiliser kubectl normalement
kubectl get pods
kubectl get services

# 6. Changer de cluster
kubectl proxyauth ctx --set another-cluster
kubectl get nodes`;
}
