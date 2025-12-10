<script setup lang="ts">
import { onMounted, nextTick, computed } from 'vue';
import { useToast } from 'maz-ui/composables/useToast';
import hljs from 'highlight.js/lib/core';
import bash from 'highlight.js/lib/languages/bash';
import yaml from 'highlight.js/lib/languages/yaml';
import 'highlight.js/styles/atom-one-dark.css';
import MazCard from 'maz-ui/components/MazCard';
import MazBtn from 'maz-ui/components/MazBtn';
import MazIcon from 'maz-ui/components/MazIcon';
import MazBadge from 'maz-ui/components/MazBadge';
import MazTabs from 'maz-ui/components/MazTabs';
import MazTabsBar from 'maz-ui/components/MazTabsBar';
import MazTabsContent from 'maz-ui/components/MazTabsContent';
import MazTabsContentItem from 'maz-ui/components/MazTabsContentItem';
import MazAccordion from 'maz-ui/components/MazAccordion';
import {
  MazCommandLine,
  MazCloudArrowDown,
  MazClipboardDocument,
  MazCodeBracket,
  MazCog6Tooth,
  MazKey,
  MazShieldCheck,
  MazDocumentText,
  MazPlay,
  MazArrowTopRightOnSquare,
  MazCube,
  MazServer
} from '@maz-ui/icons';
import { useAuthStore } from '../store/auth';

const toast = useToast();
const authStore = useAuthStore();

// Initialize highlight.js
onMounted(async () => {
  hljs.registerLanguage('bash', bash);
  hljs.registerLanguage('yaml', yaml);
  await nextTick();
  hljs.highlightAll();
});

// Helper function for external links
const openExternalLink = (url: string) => {
  window.open(url, '_blank');
};

const backendUrl = computed(() => {
  let envUrl = import.meta.env.VITE_API_BASE_URL;
  if (envUrl && (envUrl.endsWith('/') || envUrl.endsWith('/api'))) {
    envUrl = envUrl.replace(/\/api\/?$/, '').replace(/\/$/, '');
  }
  if (envUrl === '' || envUrl === undefined) {
    // IF the env variable is an empty string, use the frontend url
    envUrl = window.location.origin;
  }
  return envUrl
});

const backendUrlName = computed(() => {
  try {
    let url = backendUrl.value;
    return url.replace("http://", "").replace("https://", "").replace(".", "-").replace(":", "-");
  } catch (error) {
    return backendUrl.value;
  }
});

// Installation commands for different platforms
const installationCommands = {
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
};

// Usage examples
const usageExamples = {
  config: {
    title: 'Configuration',
    description: 'Configurer le plugin pour votre environnement',
    examples: [
      {
        title: 'Définir le serveur par défaut',
        command: `kubectl proxyauth config set-def --default-server "${backendUrlName.value}"`,
        description: 'Configure le serveur ProxyAuthK8s par défaut'
      },
      {
        title: 'Ajouter un nouveau serveur',
        command: `kubectl proxyauth login --server-url "${backendUrl.value}"`,
        description: 'Ajoute un nouveau serveur avec son URL'
      },
      {
        title: 'Configurer le namespace par défaut',
        command: `kubectl proxyauth config set-def --server "${backendUrlName.value}" --namespace "team-production"`,
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
        command: 'kubectl proxyauth login',
        description: 'Authentification globale via le navigateur'
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
};

// Complete workflow example
const workflowExample = `# 1. Configuration initiale
kubectl proxyauth config set-def --default-server "${backendUrlName.value}"
kubectl proxyauth config set-def --server "${backendUrl.value}" --namespace "default"

# 2. Authentification
kubectl proxyauth login
# Or with your current token
kubectl proxyauth login --server-url "${backendUrl.value}" --token "${authStore.getToken}"

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

// Copy to clipboard function
const copyToClipboard = async (text: string, label: string) => {
  try {
    await navigator.clipboard.writeText(text);
    toast.success(`${label} copié dans le presse-papiers !`);
  } catch (error) {
    toast.error(`Échec de la copie du ${label.toLowerCase()}.`);
    console.error('Clipboard copy failed:', error);
  }
};
</script>

<template>
  <div class="cli-view-container">
    <!-- Header Section -->
    <section class="header-section">
      <div class="header-content">
        <h1 class="page-title">
          <MazIcon :icon="MazCommandLine" size="xl" class="title-icon" />
          Plugin kubectl-proxyauth
        </h1>
        <p class="page-description">
          Guide complet d'installation et d'utilisation du plugin kubectl pour ProxyAuthK8s.
          Gérez facilement vos authentifications Kubernetes multi-clusters.
        </p>
      </div>
    </section>

    <!-- Main Content -->
    <section class="main-section">
      <div class="main-container">

        <!-- Quick Start Card -->
        <MazCard class="quick-start-card">
          <template #content-title>
            <h2 class="card-title">
              <MazIcon :icon="MazPlay" size="lg" />
              Démarrage Rapide
            </h2>
          </template>
          <template #default>
            <div class="quick-start-content">
              <div class="quick-steps">
                <div class="step">
                  <MazBadge color="primary" size="sm">1</MazBadge>
                  <span>Installer le plugin via Krew ou manuellement</span>
                </div>
                <div class="step">
                  <MazBadge color="primary" size="sm">2</MazBadge>
                  <span>Configurer le serveur ProxyAuthK8s</span>
                </div>
                <div class="step">
                  <MazBadge color="primary" size="sm">3</MazBadge>
                  <span>S'authentifier et utiliser kubectl normalement</span>
                </div>
              </div>
            </div>
          </template>
        </MazCard>

        <!-- Installation Section -->
        <MazCard class="installation-card">
          <template #content-title>
            <h2 class="card-title">
              <MazIcon :icon="MazCloudArrowDown" size="lg" />
              Installation
            </h2>
          </template>
          <template #default>
            <div class="installation-content">
              <MazTabs>
                <MazTabsBar :items="[
                  { label: installationCommands.krew.title, disabled: false },
                  { label: installationCommands.manual.title, disabled: false },
                  { label: installationCommands.homebrew.title, disabled: false }
                ]" />

                <MazTabsContent>
                  <MazTabsContentItem :tab="1">
                    <div class="installation-tab">
                      <div class="tab-header">
                        <MazBadge color="success" size="sm">{{ installationCommands.krew.badge }}</MazBadge>
                        <p class="tab-description">{{ installationCommands.krew.description }}</p>
                      </div>
                      <div class="command-block">
                        <div class="code-container">
                          <pre><code class="hljs bash">{{ installationCommands.krew.commands.join('\n') }}</code></pre>
                          <MazBtn color="primary" size="sm" :left-icon="MazClipboardDocument"
                            @click="copyToClipboard(installationCommands.krew.commands.join('\n'), 'Commandes Krew')"
                            class="copy-btn-overlay">
                            Copier
                          </MazBtn>
                        </div>
                      </div>
                    </div>
                  </MazTabsContentItem>

                  <MazTabsContentItem :tab="2">
                    <div class="installation-tab">
                      <div class="tab-header">
                        <MazBadge color="warning" size="sm">{{ installationCommands.manual.badge }}</MazBadge>
                        <p class="tab-description">{{ installationCommands.manual.description }}</p>
                      </div>
                      <div class="command-block">
                        <div class="code-container">
                          <pre><code class="hljs bash">{{ installationCommands.manual.commands.join('\n') }}</code></pre>
                          <MazBtn color="primary" size="sm" :left-icon="MazClipboardDocument"
                            @click="copyToClipboard(installationCommands.manual.commands.join('\n'), 'Commandes manuelles')"
                            class="copy-btn-overlay">
                            Copier
                          </MazBtn>
                        </div>
                      </div>
                    </div>
                  </MazTabsContentItem>

                  <MazTabsContentItem :tab="3">
                    <div class="installation-tab">
                      <div class="tab-header">
                        <MazBadge color="info" size="sm">{{ installationCommands.homebrew.badge }}</MazBadge>
                        <p class="tab-description">{{ installationCommands.homebrew.description }}</p>
                      </div>
                      <div class="command-block">
                        <div class="code-container">
                          <pre><code class="hljs bash">{{ installationCommands.homebrew.commands.join('\n') }}</code></pre>
                          <MazBtn color="primary" size="sm" :left-icon="MazClipboardDocument"
                            @click="copyToClipboard(installationCommands.homebrew.commands.join('\n'), 'Commandes Homebrew')"
                            class="copy-btn-overlay">
                            Copier
                          </MazBtn>
                        </div>
                      </div>
                    </div>
                  </MazTabsContentItem>
                </MazTabsContent>
              </MazTabs>
            </div>
          </template>
        </MazCard>

        <!-- Usage Examples -->
        <MazCard class="usage-card">
          <template #content-title>
            <h2 class="card-title">
              <MazIcon :icon="MazCodeBracket" size="lg" />
              Guide d'Utilisation
            </h2>
          </template>
          <template #default>
            <div class="usage-content">
              <MazAccordion class="usage-content-accordion">
                <template #title-1>
                  <div class="accordion-title">
                    <MazIcon :icon="MazCog6Tooth" size="lg" />
                    <span>{{ usageExamples.config.title }}</span>
                  </div>
                </template>
                <template #content-1>
                  <div class="accordion-content">
                    <p class="section-description">{{ usageExamples.config.description }}</p>
                    <div class="examples-list">
                      <div v-for="example in usageExamples.config.examples" :key="example.title" class="example-item">
                        <h4 class="example-title">{{ example.title }}</h4>
                        <p class="example-description">{{ example.description }}</p>
                        <div class="example-command">
                          <div class="example-code-container">
                            <pre><code class="hljs bash">{{ example.command }}</code></pre>
                            <MazBtn size="xs" color="primary" :left-icon="MazClipboardDocument"
                              @click="copyToClipboard(example.command, example.title)" class="example-copy-btn">
                              Copier
                            </MazBtn>
                          </div>
                        </div>
                      </div>
                    </div>
                  </div>
                </template>

                <template #title-2>
                  <div class="accordion-title">
                    <MazIcon :icon="MazKey" size="lg" />
                    <span>{{ usageExamples.auth.title }}</span>
                  </div>
                </template>
                <template #content-2>
                  <div class="accordion-content">
                    <p class="section-description">{{ usageExamples.auth.description }}</p>
                    <div class="examples-list">
                      <div v-for="example in usageExamples.auth.examples" :key="example.title" class="example-item">
                        <h4 class="example-title">{{ example.title }}</h4>
                        <p class="example-description">{{ example.description }}</p>
                        <div class="example-command">
                          <div class="example-code-container">
                            <pre><code class="hljs bash">{{ example.command }}</code></pre>
                            <MazBtn size="xs" color="primary" :left-icon="MazClipboardDocument"
                              @click="copyToClipboard(example.command, example.title)" class="example-copy-btn">
                              Copier
                            </MazBtn>
                          </div>
                        </div>
                      </div>
                    </div>
                  </div>
                </template>

                <template #title-3>
                  <div class="accordion-title">
                    <MazIcon :icon="MazServer" size="lg" />
                    <span>{{ usageExamples.clusters.title }}</span>
                  </div>
                </template>
                <template #content-3>
                  <div class="accordion-content">
                    <p class="section-description">{{ usageExamples.clusters.description }}</p>
                    <div class="examples-list">
                      <div v-for="example in usageExamples.clusters.examples" :key="example.title" class="example-item">
                        <h4 class="example-title">{{ example.title }}</h4>
                        <p class="example-description">{{ example.description }}</p>
                        <div class="example-command">
                          <div class="example-code-container">
                            <pre><code class="hljs bash">{{ example.command }}</code></pre>
                            <MazBtn size="xs" color="primary" :left-icon="MazClipboardDocument"
                              @click="copyToClipboard(example.command, example.title)" class="example-copy-btn">
                              Copier
                            </MazBtn>
                          </div>
                        </div>
                      </div>
                    </div>
                  </div>
                </template>

                <template #title-4>
                  <div class="accordion-title">
                    <MazIcon :icon="MazCube" size="lg" />
                    <span>{{ usageExamples.contexts.title }}</span>
                  </div>
                </template>
                <template #content-4>
                  <div class="accordion-content">
                    <p class="section-description">{{ usageExamples.contexts.description }}</p>
                    <div class="examples-list">
                      <div v-for="example in usageExamples.contexts.examples" :key="example.title" class="example-item">
                        <h4 class="example-title">{{ example.title }}</h4>
                        <p class="example-description">{{ example.description }}</p>
                        <div class="example-command">
                          <div class="example-code-container">
                            <pre><code class="hljs bash">{{ example.command }}</code></pre>
                            <MazBtn size="xs" color="primary" :left-icon="MazClipboardDocument"
                              @click="copyToClipboard(example.command, example.title)" class="example-copy-btn">
                              Copier
                            </MazBtn>
                          </div>
                        </div>
                      </div>
                    </div>
                  </div>
                </template>
              </MazAccordion>
            </div>
          </template>
        </MazCard>

        <!-- Complete Workflow -->
        <MazCard class="workflow-card">
          <template #content-title>
            <h2 class="card-title">
              <MazIcon :icon="MazCommandLine" size="lg" />
              Workflow Complet
            </h2>
          </template>
          <template #default>
            <div class="workflow-content">
              <p class="workflow-description">
                Exemple complet d'utilisation du plugin de l'installation à l'utilisation quotidienne.
              </p>
              <div class="workflow-command-block">
                <div class="workflow-code-container">
                  <pre><code class="hljs bash">{{ workflowExample }}</code></pre>
                  <MazBtn color="success" size="lg" :left-icon="MazClipboardDocument"
                    @click="copyToClipboard(workflowExample, 'Workflow complet')" class="workflow-copy-btn-overlay">
                    Copier le workflow complet
                  </MazBtn>
                </div>
              </div>
            </div>
          </template>
        </MazCard>

        <!-- Additional Resources -->
        <MazCard class="resources-card">
          <template #content-title>
            <h2 class="card-title">
              <MazIcon :icon="MazDocumentText" size="lg" />
              Ressources Supplémentaires
            </h2>
          </template>
          <template #default>
            <div class="resources-content">
              <div class="resources-grid">
                <div class="resource-item">
                  <MazIcon :icon="MazShieldCheck" size="lg" class="resource-icon" />
                  <h3 class="resource-title">Documentation Complète</h3>
                  <p class="resource-description">
                    Consultez le README du plugin pour plus de détails sur toutes les fonctionnalités.
                  </p>
                  <MazBtn color="primary" size="sm" :right-icon="MazArrowTopRightOnSquare"
                    @click="openExternalLink('https://github.com/batleforc/proxyauthk8s/tree/main/apps/kubectl_proxyauth')">
                    Voir le README
                  </MazBtn>
                </div>

                <div class="resource-item">
                  <MazIcon :icon="MazCloudArrowDown" size="lg" class="resource-icon" />
                  <h3 class="resource-title">Releases GitHub</h3>
                  <p class="resource-description">
                    Téléchargez les dernières versions du plugin pour tous les OS.
                  </p>
                  <MazBtn color="success" size="sm" :right-icon="MazArrowTopRightOnSquare"
                    @click="openExternalLink('https://github.com/batleforc/proxyauthk8s/releases')">
                    Voir les releases
                  </MazBtn>
                </div>

                <div class="resource-item">
                  <MazIcon :icon="MazCommandLine" size="lg" class="resource-icon" />
                  <h3 class="resource-title">Index Krew</h3>
                  <p class="resource-description">
                    Plugin disponible dans l'index officiel Krew pour une installation facile.
                  </p>
                  <MazBtn color="info" size="sm" :right-icon="MazArrowTopRightOnSquare"
                    @click="openExternalLink('https://krew.sigs.k8s.io/plugins/')">
                    Voir Krew
                  </MazBtn>
                </div>
              </div>
            </div>
          </template>
        </MazCard>

      </div>
    </section>
  </div>
</template>

<style scoped>
.cli-view-container {
  min-height: 100vh;
  background: linear-gradient(135deg, #0f172a 0%, #1e293b 100%);
  font-family: var(--font-family-sans, 'Inter', sans-serif);
}

/* Header Section */
.header-section {
  background: linear-gradient(135deg, #1e40af 0%, #7c3aed 100%);
  color: white;
  padding: 2rem 2rem 1.5rem;
  position: relative;
  overflow: hidden;
}

.header-section::before {
  content: '';
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  bottom: 0;
  background:
    radial-gradient(circle at 20% 80%, rgba(120, 119, 198, 0.4) 0%, transparent 50%),
    radial-gradient(circle at 80% 20%, rgba(255, 255, 255, 0.15) 0%, transparent 50%);
  pointer-events: none;
}

.header-content {
  max-width: 1200px;
  margin: 0 auto;
  position: relative;
  z-index: 1;
  text-align: center;
}

.usage-content-accordion {
  width: 100%;
}

.page-title {
  font-size: 2.5rem;
  font-weight: 700;
  margin: 0 0 1rem 0;
  letter-spacing: -0.025em;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 1rem;
}

.title-icon {
  color: #34d399;
}

.page-description {
  font-size: 1.25rem;
  color: rgba(255, 255, 255, 0.9);
  margin: 0;
  max-width: 800px;
  margin-left: auto;
  margin-right: auto;
  line-height: 1.6;
}

/* Main Section */
.main-section {
  padding: 3rem 2rem;
}

.main-container {
  max-width: 1200px;
  margin: 0 auto;
  display: flex;
  flex-direction: column;
  gap: 2rem;
}

/* Cards Base Styles */
.quick-start-card,
.installation-card,
.usage-card,
.workflow-card,
.resources-card {
  background: #1e293b;
  border: 1px solid rgba(255, 255, 255, 0.1);
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.3);
}

.card-title {
  font-size: 1.5rem;
  font-weight: 600;
  color: #f1f5f9;
  margin: 0;
  display: flex;
  align-items: center;
  gap: 0.75rem;
}

/* Quick Start */
.quick-start-content {
  padding: 1.5rem 0;
}

.quick-steps {
  display: flex;
  flex-direction: column;
  gap: 1rem;
}

.step {
  display: flex;
  align-items: center;
  gap: 1rem;
  padding: 1rem;
  background: rgba(255, 255, 255, 0.05);
  border-radius: 8px;
  color: #e2e8f0;
  font-size: 1.125rem;
}

/* Installation */
.installation-content {
  padding: 1.5rem 0;
}

.installation-tab {
  padding: 1rem 0;
}

.tab-header {
  display: flex;
  align-items: center;
  gap: 1rem;
  margin-bottom: 1.5rem;
}

.tab-description {
  color: #94a3b8;
  margin: 0;
  font-size: 0.875rem;
}

.command-block {
  width: 100%;
}

.command-textarea {
  flex: 1;
  font-family: 'Monaco', 'Menlo', 'Ubuntu Mono', monospace;
  font-size: 0.875rem;
}

.copy-btn {
  min-width: 80px;
}

/* Highlight.js Code Blocks */
.code-container {
  position: relative;
  border-radius: 8px;
  overflow: hidden;
  background: #282c34;
  border: 1px solid rgba(255, 255, 255, 0.1);
  width: 100%;
  max-width: 100%;
}

.code-container pre {
  margin: 0;
  padding: 1.5rem;
  background: transparent;
  overflow-x: auto;
  line-height: 1.5;
}

.code-container pre code {
  font-family: 'Monaco', 'Menlo', 'Ubuntu Mono', monospace;
  font-size: 0.875rem;
  background: transparent;
  color: #abb2bf;
}

.copy-btn-overlay {
  position: absolute;
  top: 0.75rem;
  right: 0.75rem;
  min-width: 60px;
  z-index: 10;
  opacity: 0.8;
  transition: opacity 0.2s ease;
}

.copy-btn-overlay:hover {
  opacity: 1;
}

.example-code-container {
  position: relative;
  border-radius: 6px;
  overflow: hidden;
  background: #282c34;
  border: 1px solid rgba(255, 255, 255, 0.1);
}

.example-code-container pre {
  margin: 0;
  padding: 1rem;
  background: transparent;
  overflow-x: auto;
}

.example-code-container pre code {
  font-family: 'Monaco', 'Menlo', 'Ubuntu Mono', monospace;
  font-size: 0.75rem;
  background: transparent;
  color: #abb2bf;
}

.example-copy-btn {
  position: absolute;
  top: 0.5rem;
  right: 0.5rem;
  min-width: 50px;
  z-index: 10;
}

.workflow-code-container {
  position: relative;
  border-radius: 8px;
  overflow: hidden;
  background: #282c34;
  border: 1px solid rgba(255, 255, 255, 0.1);
}

.workflow-code-container pre {
  margin: 0;
  padding: 2rem;
  background: transparent;
  overflow-x: auto;
  max-height: 400px;
  overflow-y: auto;
}

.workflow-code-container pre code {
  font-family: 'Monaco', 'Menlo', 'Ubuntu Mono', monospace;
  font-size: 0.875rem;
  background: transparent;
  color: #abb2bf;
  line-height: 1.6;
}

.workflow-copy-btn-overlay {
  position: absolute;
  top: 1rem;
  right: 1rem;
  min-width: 200px;
  z-index: 10;
  opacity: 0.9;
  transition: opacity 0.2s ease;
}

.workflow-copy-btn-overlay:hover {
  opacity: 1;
}

/* Override highlight.js theme colors for better integration */
.hljs {
  background: transparent !important;
  color: #abb2bf !important;
}

.hljs-comment {
  color: #5c6370 !important;
  font-style: italic;
}

.hljs-keyword,
.hljs-selector-tag,
.hljs-built_in {
  color: #c678dd !important;
}

.hljs-string,
.hljs-attr {
  color: #98c379 !important;
}

.hljs-number,
.hljs-literal {
  color: #d19a66 !important;
}

.hljs-variable,
.hljs-template-variable {
  color: #e06c75 !important;
}

.hljs-function .hljs-title {
  color: #61afef !important;
}

.hljs-meta {
  color: #528bff !important;
}

/* Usage Examples */
.usage-content {
  padding: 1rem 0;
}

/* Accordion Styling */
.usage-content .maz-accordion {
  width: 100%;
  max-width: 100%;
}

.usage-content .maz-accordion>div {
  width: 100%;
}

.accordion-title {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  color: #f1f5f9;
  font-weight: 500;
  width: 100%;
}

.accordion-content {
  padding: 1rem 0;
}

.section-description {
  color: #94a3b8;
  margin: 0 0 1.5rem 0;
  font-size: 0.875rem;
}

.examples-list {
  display: flex;
  flex-direction: column;
  gap: 1.5rem;
}

.example-item {
  padding: 1rem;
  background: rgba(255, 255, 255, 0.05);
  border-radius: 8px;
  border-left: 4px solid #3b82f6;
}

.example-title {
  font-size: 1rem;
  font-weight: 600;
  color: #f1f5f9;
  margin: 0 0 0.5rem 0;
}

.example-description {
  color: #94a3b8;
  margin: 0 0 1rem 0;
  font-size: 0.875rem;
}

.example-command {
  margin-top: 1rem;
}

/* Workflow */
.workflow-content {
  padding: 1.5rem 0;
}

.workflow-description {
  color: #94a3b8;
  margin: 0 0 1.5rem 0;
  font-size: 1rem;
  line-height: 1.6;
}

.workflow-command-block {
  margin-top: 1rem;
}

/* Resources */
.resources-content {
  padding: 1.5rem 0;
}

.resources-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(300px, 1fr));
  gap: 1.5rem;
}

.resource-item {
  padding: 1.5rem;
  background: rgba(255, 255, 255, 0.05);
  border-radius: 12px;
  text-align: center;
  border: 1px solid rgba(255, 255, 255, 0.1);
}

.resource-icon {
  color: #3b82f6;
  margin-bottom: 1rem;
}

.resource-title {
  font-size: 1.125rem;
  font-weight: 600;
  color: #f1f5f9;
  margin: 0 0 0.75rem 0;
}

.resource-description {
  color: #94a3b8;
  margin: 0 0 1.5rem 0;
  font-size: 0.875rem;
  line-height: 1.5;
}

/* Responsive Design */
@media (max-width: 768px) {
  .page-title {
    font-size: 2rem;
    flex-direction: column;
    gap: 0.5rem;
  }

  .installation-tab {
    padding: 0.5rem 0;
  }

  .tab-header {
    flex-direction: column;
    align-items: flex-start;
    gap: 0.5rem;
    margin-bottom: 1rem;
  }

  .copy-btn-overlay {
    position: static;
    margin-top: 1rem;
    width: 100%;
  }

  .code-container {
    overflow-x: auto;
  }

  .code-container pre {
    padding: 1rem;
    font-size: 0.75rem;
  }

  .example-command {
    flex-direction: column;
    align-items: flex-start;
  }

  .example-copy-btn {
    align-self: flex-end;
  }

  .resources-grid {
    grid-template-columns: 1fr;
  }

  .quick-steps {
    gap: 0.75rem;
  }

  .step {
    font-size: 1rem;
  }
}

@media (max-width: 480px) {
  .main-section {
    padding: 2rem 1rem;
  }

  .header-section {
    padding: 1.5rem 1rem 1rem;
  }

  .page-title {
    font-size: 1.75rem;
  }

  .page-description {
    font-size: 1rem;
  }

  .installation-content {
    padding: 1rem 0;
  }

  .code-container pre code {
    font-size: 0.7rem;
  }

  .copy-btn-overlay {
    font-size: 0.75rem;
    padding: 0.5rem;
  }
}
</style>
