<script setup lang="ts">
import { computed, onMounted, ref } from 'vue';
import { useClustersStore } from '../store/clusters.ts';
import { useAuthStore } from '../store/auth.ts';
import { useToast } from 'maz-ui/composables/useToast';
import MazCard from 'maz-ui/components/MazCard';
import MazBtn from 'maz-ui/components/MazBtn';
import MazIcon from 'maz-ui/components/MazIcon';
import MazBadge from 'maz-ui/components/MazBadge';
import MazSpinner from 'maz-ui/components/MazSpinner';
import MazTextarea from 'maz-ui/components/MazTextarea';
import MazTabs from 'maz-ui/components/MazTabs';
import MazTabsBar from 'maz-ui/components/MazTabsBar';
import MazTabsContent from 'maz-ui/components/MazTabsContent';
import MazTabsContentItem from 'maz-ui/components/MazTabsContentItem';
import {
  MazServer,
  MazShieldCheck,
  MazArrowTopRightOnSquare,
  MazClipboardDocument,
  MazCloudArrowDown,
  MazKey,
  MazGlobeAlt,
  MazClock,
  MazCog6Tooth
} from '@maz-ui/icons';

const clustersStore = useClustersStore();
const authStore = useAuthStore();
const toast = useToast();

// État de l'authentification
const isAuthenticating = ref(true);
const authenticationComplete = ref(false);

// Lifecycle
onMounted(async () => {
  console.info(`Well it does not work`, { inited: authStore.inited, user: authStore.user, cluster: clustersStore.inited });
  if (authStore.isInited && authStore.isAuthenticated && clustersStore.inited === false) {
    toast.info('Récupération des clusters en cours...', { duration: 3000 });
    await clustersStore.fetchClusters(toast);
  }
  if (authStore.isInited && authStore.isAuthenticated && clustersStore.inited) {
    try {
      toast.info('Finalisation de l\'authentification...', { duration: 3000 });
      await clustersStore.callBackFromCluster(toast);
      authenticationComplete.value = true;
    } catch (error) {
      toast.error('Erreur lors de la finalisation de l\'authentification.', { duration: 5000 });
      console.error('Error during callback:', error);
    } finally {
      isAuthenticating.value = false;
    }
  }
});


// Données calculées
const callbackData = computed(() => clustersStore.callBack);
const hasToken = computed(() => !!callbackData.value.retour?.access_token);

// Génération du kubeconfig
const generateKubeconfig = () => {
  const data = callbackData.value;
  if (!data.retour?.access_token) return '';

  const clusterName = `${data.ns}-${data.cluster}`;
  const userName = `${data.retour.subject}@${data.ns}-${data.cluster}`;
  const contextName = `${data.ns}-${data.cluster}-context`;

  const kubeconfigYaml = `apiVersion: v1
kind: Config
clusters:
- name: ${clusterName}
  cluster:
    server: ${data.retour.cluster_url}
    insecure-skip-tls-verify: false
users:
- name: ${userName}
  user:
    token: ${data.retour.id_token}
contexts:
- name: ${contextName}
  context:
    cluster: ${clusterName}
    user: ${userName}
current-context: ${contextName}`;

  return kubeconfigYaml;
};

const generatePluginKubeconfig = () => {
  const data = callbackData.value;
  if (!data.retour?.access_token) return '';

  const clusterName = `${data.ns}-${data.cluster}`;
  const userName = `${data.retour.subject}@${data.ns}-${data.cluster}`;
  const contextName = `${data.ns}-${data.cluster}-context`;
  // create a const of the cluster_url without the path
  const clusterUrl = new URL(data.retour.cluster_url).origin;

  const kubeconfigYaml = `apiVersion: v1
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
        - -n ${data.ns}
        - -s ${clusterUrl}
        - ${data.cluster}
      command: kubectl
      env: null
      provideClusterInfo: false
contexts:
- name: ${contextName}
  context:
    cluster: ${clusterName}
    user: ${userName}
current-context: ${contextName}`;

  return kubeconfigYaml;
};

const generatePluginCommands = () => {
  const data = callbackData.value;
  if (!data.retour?.access_token) return '';

  const clusterUrl = new URL(data.retour.cluster_url).origin;

  return `# 1. S'authentifier avec votre token actuel
kubectl proxyauth login --server-url "${clusterUrl}" --token "${data.retour.access_token}"

# 2. Se connecter au cluster spécifique
kubectl proxyauth login "${data.cluster}"
# Ou via votre token
kubectl proxyauth login "${data.cluster}" --token "${callbackData.value.retour.id_token}"

# 3. Utiliser kubectl normalement
kubectl get pods
kubectl get services

# 4. Optionnel: Changer le contexte kubectl vers ce cluster
kubectl proxyauth ctx --set "${data.cluster}"`;
};

// Actions
const copyToClipboard = async (text: string, label: string) => {
  try {
    await navigator.clipboard.writeText(text);
    toast.success(`${label} copié dans le presse-papiers !`);
  } catch (error) {
    toast.error(`Échec de la copie du ${label.toLowerCase()}.`);
    console.error('Clipboard copy failed:', error);
  }
};

const downloadKubeconfig = () => {
  const kubeconfigContent = generateKubeconfig();
  const blob = new Blob([kubeconfigContent], { type: 'application/yaml' });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = `kubeconfig-${callbackData.value.ns}-${callbackData.value.cluster}.yaml`;
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
  toast.success('Kubeconfig téléchargé avec succès !');
};

const downloadPluginKubeconfig = () => {
  const kubeconfigContent = generatePluginKubeconfig();
  const blob = new Blob([kubeconfigContent], { type: 'application/yaml' });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = `kubeconfig-plugin-${callbackData.value.ns}-${callbackData.value.cluster}.yaml`;
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
  toast.success('Kubeconfig Plugin téléchargé avec succès !');
};


</script>

<template>
  <div class="cluster-callback-container">
    <!-- Header Section -->
    <section class="header-section">
      <div class="header-content">
        <h1 class="page-title">
          <MazIcon :icon="MazShieldCheck" size="lg" class="title-icon" />
          Authentification Cluster
        </h1>
        <p class="page-description">
          Finalisation de l'authentification pour le cluster
          <strong>{{ callbackData.cluster }}</strong>
          dans le namespace <strong>{{ callbackData.ns }}</strong>
        </p>
      </div>
    </section>

    <!-- Main Content -->
    <section class="main-section">
      <div class="main-container">

        <!-- Étape 1: Attente de récupération du token -->
        <div v-if="isAuthenticating" class="waiting-step">
          <MazCard class="waiting-card">
            <template #default>
              <div class="waiting-content">
                <MazSpinner size="3rem" color="primary" class="waiting-spinner" />
                <h2 class="waiting-title">
                  <MazIcon :icon="MazClock" size="lg" />
                  Récupération des tokens en cours...
                </h2>
                <p class="waiting-description">
                  Nous récupérons vos tokens d'authentification auprès du provider d'identité.
                  Veuillez patienter quelques instants.
                </p>
                <div class="waiting-details">
                  <div class="detail-item">
                    <MazIcon :icon="MazServer" size="sm" class="detail-icon" />
                    <span>Cluster: {{ callbackData.cluster || 'Chargement...' }}</span>
                  </div>
                  <div class="detail-item">
                    <MazIcon :icon="MazGlobeAlt" size="sm" class="detail-icon" />
                    <span>Namespace: {{ callbackData.ns || 'Chargement...' }}</span>
                  </div>
                </div>
              </div>
            </template>
          </MazCard>
        </div>

        <!-- Étape 2: Affichage des tokens et informations -->
        <div v-else-if="authenticationComplete && hasToken" class="success-step">
          <div class="success-grid">

            <!-- Informations du cluster -->
            <MazCard class="info-card">
              <template #content-title>
                <h3 class="card-title">
                  <MazIcon :icon="MazServer" size="lg" />
                  Informations du Cluster
                </h3>
              </template>
              <template #default>
                <div class="info-content">
                  <div class="info-row">
                    <span class="info-label">Nom du cluster:</span>
                    <MazBadge color="info" size="sm">{{ callbackData.cluster }}</MazBadge>
                  </div>
                  <div class="info-row">
                    <span class="info-label">Namespace:</span>
                    <MazBadge color="primary" size="sm">{{ callbackData.ns }}</MazBadge>
                  </div>
                  <div class="info-row">
                    <span class="info-label">URL de l'API:</span>
                    <span class="info-value">{{ callbackData.retour.cluster_url }}</span>
                  </div>
                  <div class="info-row">
                    <span class="info-label">Utilisateur:</span>
                    <span class="info-value">{{ callbackData.retour.subject }}</span>
                  </div>
                </div>
              </template>
            </MazCard>

            <!-- Tokens d'authentification -->
            <MazCard class="tokens-card">
              <template #content-title>
                <h3 class="card-title">
                  <MazIcon :icon="MazKey" size="lg" />
                  Tokens d'Authentification
                </h3>
              </template>
              <template #default>
                <div class="tokens-content">
                  <div class="token-section">
                    <label for="access-token" class="token-label">Access Token:</label>
                    <div class="token-field">
                      <MazTextarea id="access-token" :model-value="callbackData.retour.access_token" readonly :rows="3"
                        class="token-textarea" />
                      <MazBtn size="sm" color="primary" :left-icon="MazClipboardDocument"
                        @click="copyToClipboard(callbackData.retour.access_token, 'Access Token')" class="copy-button">
                        Copier
                      </MazBtn>
                    </div>
                  </div>

                  <div class="token-section">
                    <label for="id-token" class="token-label">ID Token:</label>
                    <div class="token-field">
                      <MazTextarea id="id-token" :model-value="callbackData.retour.id_token" readonly :rows="3"
                        class="token-textarea" />
                      <MazBtn size="sm" color="primary" :left-icon="MazClipboardDocument"
                        @click="copyToClipboard(callbackData.retour.id_token, 'ID Token')" class="copy-button">
                        Copier
                      </MazBtn>
                    </div>
                  </div>

                  <div v-if="callbackData.retour.refresh_token !== ''" class="token-section">
                    <label for="refresh-token" class="token-label">Refresh Token:</label>
                    <div class="token-field">
                      <MazTextarea id="refresh-token" :model-value="callbackData.retour.refresh_token" readonly
                        :rows="3" class="token-textarea" />
                      <MazBtn size="sm" color="primary" :left-icon="MazClipboardDocument"
                        @click="copyToClipboard(callbackData.retour.refresh_token, 'Refresh Token')"
                        class="copy-button">
                        Copier
                      </MazBtn>
                    </div>
                  </div>
                </div>
              </template>
            </MazCard>

            <!-- Kubeconfig -->
            <MazCard class="kubeconfig-card full-width">
              <template #content-title>
                <h3 class="card-title">
                  <MazIcon :icon="MazCloudArrowDown" size="lg" />
                  Configuration Kubernetes
                </h3>
              </template>
              <template #default>
                <div class="kubeconfig-content">
                  <p class="kubeconfig-description">
                    Choisissez le type de configuration Kubernetes selon votre usage.
                  </p>

                  <MazTabs>
                    <MazTabsBar :items="[
                      { label: 'Kubeconfig Standard', disabled: false },
                      { label: 'Kubeconfig Plugin', disabled: false },
                      { label: 'Configuration via Plugin CLI', disabled: false }
                    ]" />

                    <MazTabsContent>
                      <MazTabsContentItem :tab="1">
                        <div class="tab-content">
                          <p class="tab-description">
                            Configuration avec token statique. Utilisez ce fichier kubeconfig pour vous connecter
                            directement avec le token fourni.
                          </p>

                          <div class="kubeconfig-field" v-highlight>
                            <pre><code class="hljs yaml">{{ generateKubeconfig() }}</code></pre>
                          </div>

                          <div class="kubeconfig-actions">
                            <MazBtn color="success" size="lg" :left-icon="MazCloudArrowDown" @click="downloadKubeconfig"
                              class="download-button">
                              Télécharger kubeconfig
                            </MazBtn>
                            <MazBtn color="primary" size="lg" :left-icon="MazClipboardDocument"
                              @click="copyToClipboard(generateKubeconfig(), 'Kubeconfig')"
                              class="copy-kubeconfig-button">
                              Copier kubeconfig
                            </MazBtn>
                          </div>
                        </div>
                      </MazTabsContentItem>

                      <MazTabsContentItem :tab="2">
                        <div class="tab-content">
                          <p class="tab-description">
                            Configuration avec plugin kubectl-proxyauth. Les tokens sont automatiquement gérés et
                            rafraîchis par le plugin.
                          </p>

                          <div class="kubeconfig-field" v-highlight>
                            <pre><code class="hljs yaml">{{ generatePluginKubeconfig() }}</code></pre>
                          </div>

                          <div class="kubeconfig-actions">
                            <MazBtn color="success" size="lg" :left-icon="MazCloudArrowDown"
                              @click="downloadPluginKubeconfig" class="download-button">
                              Télécharger kubeconfig plugin
                            </MazBtn>
                            <MazBtn color="primary" size="lg" :left-icon="MazClipboardDocument"
                              @click="copyToClipboard(generatePluginKubeconfig(), 'Kubeconfig Plugin')"
                              class="copy-kubeconfig-button">
                              Copier kubeconfig plugin
                            </MazBtn>
                          </div>
                        </div>
                      </MazTabsContentItem>

                      <MazTabsContentItem :tab="3">
                        <div class="tab-content">
                          <p class="tab-description">
                            Utilisez le plugin kubectl-proxyauth pour vous connecter directement via la ligne de
                            commande.
                            Cette méthode est recommandée pour une intégration transparente avec kubectl.
                          </p>

                          <div class="cli-steps">
                            <div class="cli-step">
                              <h4 class="cli-step-title">
                                <MazIcon :icon="MazCloudArrowDown" size="sm" />
                                Étape 1: Installer le plugin (si pas déjà fait)
                              </h4>
                              <div class="cli-code-container" v-highlight>
                                <pre><code class="hljs bash"># Via Krew (recommandé)
kubectl krew install proxyauth

# Ou téléchargement manuel
wget https://github.com/batleforc/proxyauthk8s/releases/latest/download/kubectl-proxyauth-linux-amd64
chmod +x kubectl-proxyauth-linux-amd64
sudo mv kubectl-proxyauth-linux-amd64 /usr/local/bin/kubectl-proxyauth</code></pre>
                                <MazBtn size="sm" color="primary" :left-icon="MazClipboardDocument"
                                  @click="copyToClipboard('kubectl krew install proxyauth', 'Commande d\'installation')"
                                  class="cli-copy-btn">
                                  Copier installation
                                </MazBtn>
                              </div>
                            </div>

                            <div class="cli-step">
                              <h4 class="cli-step-title">
                                <MazIcon :icon="MazCog6Tooth" size="sm" />
                                Étape 2: Configurer et se connecter
                              </h4>
                              <div class="cli-code-container" v-highlight>
                                <pre><code class="hljs bash">{{ generatePluginCommands() }}</code></pre>
                                <MazBtn size="sm" color="primary" :left-icon="MazClipboardDocument"
                                  @click="copyToClipboard(generatePluginCommands(), 'Commandes CLI')"
                                  class="cli-copy-btn">
                                  Copier commandes
                                </MazBtn>
                              </div>
                            </div>

                            <div class="cli-step">
                              <h4 class="cli-step-title">
                                <MazIcon :icon="MazShieldCheck" size="sm" />
                                Étape 3: Vérifier la configuration
                              </h4>
                              <div class="cli-code-container" v-highlight>
                                <pre><code class="hljs bash"># Vérifier la liste des clusters disponibles
kubectl proxyauth get

# Vérifier le contexte actuel
kubectl proxyauth ctx

# Tester la connexion
kubectl get nodes</code></pre>
                                <MazBtn size="sm" color="primary" :left-icon="MazClipboardDocument"
                                  @click="copyToClipboard('kubectl proxyauth get\nkubectl proxyauth ctx\nkubectl get nodes', 'Commandes de vérification')"
                                  class="cli-copy-btn">
                                  Copier vérification
                                </MazBtn>
                              </div>
                            </div>
                          </div>

                          <div class="cli-advantages">
                            <h4 class="advantages-title">Avantages du plugin CLI:</h4>
                            <ul class="advantages-list">
                              <li>🔄 Gestion automatique du renouvellement des tokens</li>
                              <li>🚀 Intégration native avec kubectl</li>
                              <li>⚙️ Configuration centralisée des clusters</li>
                              <li>🔐 Authentification sécurisée via navigateur</li>
                              <li>📋 Gestion des contextes kubectl simplifiée</li>
                            </ul>
                          </div>
                        </div>
                      </MazTabsContentItem>
                    </MazTabsContent>
                  </MazTabs>
                </div>
              </template>
            </MazCard>
          </div>
        </div>

        <!-- État d'erreur -->
        <div v-else class="error-step">
          <MazCard class="error-card">
            <template #default>
              <div class="error-content">
                <MazIcon :icon="MazArrowTopRightOnSquare" size="xl" class="error-icon" />
                <h2 class="error-title">Authentification échouée</h2>
                <p class="error-description">
                  Une erreur s'est produite lors de la récupération des tokens d'authentification.
                  Veuillez réessayer ou contacter votre administrateur.
                </p>
                <MazBtn color="primary" size="lg" @click="$router.push({ name: 'home' })" class="back-button">
                  Retour à l'accueil
                </MazBtn>
              </div>
            </template>
          </MazCard>
        </div>

      </div>
    </section>
  </div>
</template>

<style scoped>
.cluster-callback-container {
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

.page-title {
  font-size: 2rem;
  font-weight: 700;
  margin: 0 0 0.75rem 0;
  letter-spacing: -0.025em;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 0.75rem;
}

.title-icon {
  color: #34d399;
}

.page-description {
  font-size: 1.125rem;
  color: rgba(255, 255, 255, 0.9);
  margin: 0;
  max-width: 700px;
  margin-left: auto;
  margin-right: auto;
}

/* Main Section */
.main-section {
  padding: 3rem 2rem;
}

.main-container {
  max-width: 1200px;
  margin: 0 auto;
}

/* Waiting Step */
.waiting-card {
  background: #1e293b;
  border: 1px solid rgba(255, 255, 255, 0.1);
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.3);
}

.waiting-content {
  text-align: center;
  padding: 3rem 2rem;
}

.waiting-spinner {
  margin-bottom: 2rem;
}

.waiting-title {
  font-size: 1.5rem;
  font-weight: 600;
  color: #f1f5f9;
  margin: 0 0 1rem 0;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 0.5rem;
}

.waiting-description {
  font-size: 1.125rem;
  color: #94a3b8;
  margin: 0 0 2rem 0;
  line-height: 1.6;
}

.waiting-details {
  display: flex;
  justify-content: center;
  gap: 2rem;
  flex-wrap: wrap;
}

.detail-item {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  color: #cbd5e1;
  font-size: 0.875rem;
}

.detail-icon {
  color: #a78bfa;
}

/* Success Step */
.success-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(400px, 1fr));
  gap: 1.5rem;
}

.full-width {
  grid-column: 1 / -1;
}

.info-card,
.tokens-card,
.kubeconfig-card {
  background: #1e293b;
  border: 1px solid rgba(255, 255, 255, 0.1);
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.3);
}

.card-title {
  font-size: 1.25rem;
  font-weight: 600;
  color: #f1f5f9;
  margin: 0;
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

/* Info Card */
.info-content {
  padding: 1rem 0;
}

.info-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 1rem;
  padding: 0.75rem;
  background: rgba(255, 255, 255, 0.05);
  border-radius: 8px;
}

.info-label {
  font-weight: 500;
  color: #cbd5e1;
}

.info-value {
  color: #f1f5f9;
  font-family: 'Monaco', 'Menlo', 'Ubuntu Mono', monospace;
  font-size: 0.875rem;
  word-break: break-all;
}

/* Tokens Card */
.tokens-content {
  padding: 1rem 0;
}

.token-section {
  margin-bottom: 1.5rem;
}

.token-label {
  display: block;
  font-weight: 500;
  color: #cbd5e1;
  margin-bottom: 0.5rem;
}

.token-field {
  display: flex;
  gap: 0.5rem;
  align-items: flex-start;
}

.token-textarea {
  flex: 1;
  font-family: 'Monaco', 'Menlo', 'Ubuntu Mono', monospace;
  font-size: 0.75rem;
}

.copy-button {
  min-width: 80px;
}

/* Kubeconfig Card */
.kubeconfig-content {
  padding: 1rem 0;
}

.kubeconfig-description {
  color: #94a3b8;
  margin: 0 0 1.5rem 0;
  line-height: 1.6;
}

.tab-content {
  padding: 1rem 0;
}

.tab-description {
  color: #94a3b8;
  margin: 0 0 1rem 0;
  line-height: 1.6;
  font-size: 0.875rem;
}

.kubeconfig-field {
  margin-bottom: 1.5rem;
}

/* Kubeconfig Code Blocks */
.kubeconfig-code-container {
  border-radius: 8px;
  overflow: hidden;
  background: #282c34;
  border: 1px solid rgba(255, 255, 255, 0.1);
  max-height: 400px;
  overflow-y: auto;
}

.kubeconfig-code-container pre {
  margin: 0;
  padding: 1.5rem;
  background: transparent;
  overflow-x: auto;
  line-height: 1.5;
}

.kubeconfig-code-container pre code {
  font-family: 'Monaco', 'Menlo', 'Ubuntu Mono', monospace;
  font-size: 0.75rem;
  background: transparent;
  color: #abb2bf;
}

/* Override highlight.js theme colors for kubeconfig */
.kubeconfig-code-container .hljs {
  background: transparent !important;
  color: #abb2bf !important;
}

.kubeconfig-code-container .hljs-comment {
  color: #5c6370 !important;
  font-style: italic;
}

.kubeconfig-code-container .hljs-keyword,
.kubeconfig-code-container .hljs-selector-tag,
.kubeconfig-code-container .hljs-built_in {
  color: #c678dd !important;
}

.kubeconfig-code-container .hljs-string,
.kubeconfig-code-container .hljs-attr {
  color: #98c379 !important;
}

.kubeconfig-code-container .hljs-number,
.kubeconfig-code-container .hljs-literal {
  color: #d19a66 !important;
}

.kubeconfig-code-container .hljs-variable,
.kubeconfig-code-container .hljs-template-variable {
  color: #e06c75 !important;
}

.kubeconfig-code-container .hljs-function .hljs-title {
  color: #61afef !important;
}

.kubeconfig-code-container .hljs-meta {
  color: #528bff !important;
}

.kubeconfig-actions {
  display: flex;
  gap: 1rem;
  flex-wrap: wrap;
}

.download-button,
.copy-kubeconfig-button {
  min-width: 180px;
}

/* Error Step */
.error-card {
  background: #1e293b;
  border: 1px solid rgba(239, 68, 68, 0.3);
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.3);
}

.error-content {
  text-align: center;
  padding: 3rem 2rem;
}

.error-icon {
  color: #ef4444;
  margin-bottom: 1.5rem;
}

.error-title {
  font-size: 1.5rem;
  font-weight: 600;
  color: #f1f5f9;
  margin: 0 0 1rem 0;
}

.error-description {
  font-size: 1.125rem;
  color: #94a3b8;
  margin: 0 0 2rem 0;
  line-height: 1.6;
}

.back-button {
  min-width: 160px;
}

/* CLI Steps */
.cli-steps {
  display: flex;
  flex-direction: column;
  gap: 2rem;
  margin-bottom: 2rem;
}

.cli-step {
  padding: 1.5rem;
  background: rgba(255, 255, 255, 0.05);
  border-radius: 12px;
  border-left: 4px solid #3b82f6;
}

.cli-step-title {
  font-size: 1.125rem;
  font-weight: 600;
  color: #f1f5f9;
  margin: 0 0 1rem 0;
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

.cli-code-container {
  position: relative;
  border-radius: 8px;
  overflow: hidden;
  background: #282c34;
  border: 1px solid rgba(255, 255, 255, 0.1);
  margin-bottom: 1rem;
}

.cli-code-container pre {
  margin: 0;
  padding: 1.5rem;
  background: transparent;
  overflow-x: auto;
  line-height: 1.5;
}

.cli-code-container pre code {
  font-family: 'Monaco', 'Menlo', 'Ubuntu Mono', monospace;
  font-size: 0.875rem;
  background: transparent;
  color: #abb2bf;
}

.cli-copy-btn {
  position: absolute;
  top: 0.75rem;
  right: 0.75rem;
  min-width: 120px;
  z-index: 10;
  opacity: 0.8;
  transition: opacity 0.2s ease;
}

.cli-copy-btn:hover {
  opacity: 1;
}

/* CLI Advantages */
.cli-advantages {
  padding: 1.5rem;
  background: rgba(34, 197, 94, 0.1);
  border-radius: 12px;
  border-left: 4px solid #22c55e;
}

.advantages-title {
  font-size: 1.125rem;
  font-weight: 600;
  color: #f1f5f9;
  margin: 0 0 1rem 0;
}

.advantages-list {
  list-style: none;
  padding: 0;
  margin: 0;
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
}

.advantages-list li {
  color: #cbd5e1;
  font-size: 0.875rem;
  line-height: 1.5;
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

/* Override highlight.js theme colors for CLI code blocks */
.cli-code-container .hljs {
  background: transparent !important;
  color: #abb2bf !important;
}

.cli-code-container .hljs-comment {
  color: #5c6370 !important;
  font-style: italic;
}

.cli-code-container .hljs-keyword,
.cli-code-container .hljs-selector-tag,
.cli-code-container .hljs-built_in {
  color: #c678dd !important;
}

.cli-code-container .hljs-string,
.cli-code-container .hljs-attr {
  color: #98c379 !important;
}

.cli-code-container .hljs-number,
.cli-code-container .hljs-literal {
  color: #d19a66 !important;
}

.cli-code-container .hljs-variable,
.cli-code-container .hljs-template-variable {
  color: #e06c75 !important;
}

.cli-code-container .hljs-function .hljs-title {
  color: #61afef !important;
}

.cli-code-container .hljs-meta {
  color: #528bff !important;
}

/* Responsive Design */
@media (max-width: 768px) {
  .page-title {
    font-size: 1.75rem;
    flex-direction: column;
    gap: 0.5rem;
  }

  .success-grid {
    grid-template-columns: 1fr;
  }

  .token-field {
    flex-direction: column;
  }

  .copy-button {
    align-self: flex-start;
  }

  .kubeconfig-actions {
    flex-direction: column;
  }

  .download-button,
  .copy-kubeconfig-button {
    width: 100%;
  }

  .waiting-details {
    flex-direction: column;
    gap: 1rem;
  }

  .cli-copy-btn {
    position: static;
    margin-top: 1rem;
    width: 100%;
  }

  .cli-step {
    padding: 1rem;
  }

  .cli-steps {
    gap: 1.5rem;
  }
}

@media (max-width: 480px) {
  .main-section {
    padding: 2rem 1rem;
  }

  .header-section {
    padding: 1.5rem 1rem 1rem;
  }

  .waiting-content {
    padding: 2rem 1rem;
  }
}
</style>
