<script setup lang="ts">
import { computed, onMounted, ref } from 'vue';
import { useClustersStore } from '../store/clusters.ts';
import { useAuthStore } from '../store/auth.ts';
import { useToast } from 'maz-ui/composables/useToast';
import { useRoute, useRouter } from 'vue-router';
import MazCard from 'maz-ui/components/MazCard';
import MazBtn from 'maz-ui/components/MazBtn';
import MazIcon from 'maz-ui/components/MazIcon';
import MazBadge from 'maz-ui/components/MazBadge';
import MazSpinner from 'maz-ui/components/MazSpinner';
import {
  LazyMazServer,
  LazyMazShieldCheck,
  LazyMazArrowTopRightOnSquare,
  LazyMazClipboardDocument,
  LazyMazCloudArrowDown,
  LazyMazClock,
  LazyMazInformationCircle
} from '@maz-ui/icons';

const clustersStore = useClustersStore();
const authStore = useAuthStore();
const toast = useToast();
const route = useRoute();
const router = useRouter();

// État de chargement
const isLoading = ref(true);
const clusterData = ref<any>(null);

// Lifecycle
onMounted(async () => {
  const ns = route.params.ns as string;
  const cluster = route.params.cluster as string;

  if (!ns || !cluster) {
    toast.error('Paramètres manquants dans l\'URL', { duration: 5000 });
    setTimeout(() => {
      router.push({ name: 'home' });
    }, 2000);
    return;
  }

  // Vérifier si les clusters sont chargés
  if (!clustersStore.isInited) {
    toast.info('Chargement des clusters...', { duration: 3000 });
    await clustersStore.fetchClusters(toast);
  }

  // Trouver le cluster
  const foundCluster = clustersStore.getClusters.find(
    c => c.namespace === ns && c.name === cluster
  );

  if (!foundCluster) {
    toast.error('Cluster introuvable', { duration: 5000 });
    setTimeout(() => {
      router.push({ name: 'home' });
    }, 2000);
    return;
  }

  if (foundCluster.sso_enabled) {
    toast.warning('Ce cluster utilise SSO, redirection vers la page appropriée...', { duration: 3000 });
    setTimeout(() => {
      router.push({ name: 'home' });
    }, 2000);
    return;
  }

  clusterData.value = foundCluster;
  isLoading.value = false;
});

// Données calculées
const clusterUrl = computed(() => {
  if (!clusterData.value) return '';
  // L'URL du cluster via le proxy
  return `${globalThis.location.origin}/clusters/${clusterData.value.namespace}/${clusterData.value.name}`;
});

// Génération du kubeconfig
const generateKubeconfig = () => {
  if (!clusterData.value) return '';

  const clusterName = `${clusterData.value.namespace}-${clusterData.value.name}`;
  const userName = `${authStore.user?.profile?.preferred_username || 'user'}@${clusterData.value.namespace}-${clusterData.value.name}`;
  const contextName = `${clusterData.value.namespace}-${clusterData.value.name}-context`;

  const kubeconfigYaml = `apiVersion: v1
kind: Config
clusters:
- name: ${clusterName}
  cluster:
    server: ${clusterUrl.value}
    insecure-skip-tls-verify: false
users:
- name: ${userName}
  user:
    token: YOUR_ACCESS_TOKEN
contexts:
- name: ${contextName}
  context:
    cluster: ${clusterName}
    user: ${userName}
current-context: ${contextName}`;

  return kubeconfigYaml;
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
  a.download = `kubeconfig-${clusterData.value.namespace}-${clusterData.value.name}.yaml`;
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
  toast.success('Kubeconfig téléchargé avec succès !');
};
</script>

<template>
  <div class="cluster-nosso-container">
    <!-- Header Section -->
    <section class="header-section">
      <div class="header-content">
        <h1 class="page-title">
          <MazIcon :icon="LazyMazServer" size="lg" class="title-icon" />
          Cluster sans SSO
        </h1>
        <p class="page-description">
          Configuration pour le cluster
          <strong>{{ clusterData?.name || '...' }}</strong>
          dans le namespace <strong>{{ clusterData?.namespace || '...' }}</strong>
        </p>
      </div>
    </section>

    <!-- Main Content -->
    <section class="main-section">
      <div class="main-container">

        <!-- Chargement -->
        <div v-if="isLoading" class="loading-step">
          <MazCard class="loading-card">
            <template #default>
              <div class="loading-content">
                <MazSpinner size="3rem" color="primary" class="loading-spinner" />
                <h2 class="loading-title">
                  <MazIcon :icon="LazyMazClock" size="lg" />
                  Chargement des informations du cluster...
                </h2>
                <p class="loading-description">
                  Récupération des informations du cluster. Veuillez patienter.
                </p>
              </div>
            </template>
          </MazCard>
        </div>

        <!-- Affichage du cluster -->
        <div v-else-if="clusterData" class="cluster-step">
          <div class="cluster-grid">

            <!-- Information box -->
            <MazCard class="info-box full-width">
              <template #content-title>
                <h3 class="card-title">
                  <MazIcon :icon="LazyMazInformationCircle" size="lg" />
                  À propos de ce cluster
                </h3>
              </template>
              <template #default>
                <div class="info-box-content">
                  <p class="info-message">
                    Ce cluster n'utilise pas d'authentification SSO/OIDC. L'accès se fait via le proxy ProxyAuth
                    en utilisant votre token d'authentification actuel. Le kubeconfig généré utilisera le proxy
                    comme point d'entrée pour toutes les requêtes vers le cluster.
                  </p>
                </div>
              </template>
            </MazCard>

            <!-- Informations du cluster -->
            <MazCard class="info-card">
              <template #content-title>
                <h3 class="card-title">
                  <MazIcon :icon="LazyMazServer" size="lg" />
                  Informations du Cluster
                </h3>
              </template>
              <template #default>
                <div class="info-content">
                  <div class="info-row">
                    <span class="info-label">Nom du cluster:</span>
                    <MazBadge color="info" size="sm">{{ clusterData.name }}</MazBadge>
                  </div>
                  <div class="info-row">
                    <span class="info-label">Namespace:</span>
                    <MazBadge color="primary" size="sm">{{ clusterData.namespace }}</MazBadge>
                  </div>
                  <div class="info-row">
                    <span class="info-label">URL du proxy:</span>
                    <span class="info-value">{{ clusterUrl }}</span>
                  </div>
                  <div class="info-row">
                    <span class="info-label">Utilisateur:</span>
                    <span class="info-value">{{ authStore.user?.profile?.preferred_username || 'N/A' }}</span>
                  </div>
                  <div class="info-row">
                    <span class="info-label">Statut:</span>
                    <MazBadge :color="clusterData.enabled ? 'success' : 'danger'" size="sm">
                      {{ clusterData.enabled ? 'Activé' : 'Désactivé' }}
                    </MazBadge>
                  </div>
                  <div class="info-row" v-if="clusterData.is_reachable !== null">
                    <span class="info-label">Accessible:</span>
                    <MazBadge :color="clusterData.is_reachable ? 'success' : 'warning'" size="sm">
                      {{ clusterData.is_reachable ? 'Oui' : 'Non' }}
                    </MazBadge>
                  </div>
                </div>
              </template>
            </MazCard>

            <!-- Authentication Info -->
            <MazCard class="auth-card">
              <template #content-title>
                <h3 class="card-title">
                  <MazIcon :icon="LazyMazShieldCheck" size="lg" />
                  Authentification
                </h3>
              </template>
              <template #default>
                <div class="auth-content">
                  <p class="auth-description">
                    L'accès à ce cluster se fait via votre token d'authentification ProxyAuth.
                    Toutes les requêtes passent par le proxy qui gère l'authentification.
                  </p>
                  <div class="auth-details">
                    <div class="auth-item">
                      <MazIcon :icon="LazyMazShieldCheck" size="sm" class="auth-icon" />
                      <span>Pas de redirections SSO requises</span>
                    </div>
                    <div class="auth-item">
                      <MazIcon :icon="LazyMazServer" size="sm" class="auth-icon" />
                      <span>Requêtes via le proxy ProxyAuth</span>
                    </div>
                    <div class="auth-item">
                      <MazIcon :icon="LazyMazShieldCheck" size="sm" class="auth-icon" />
                      <span>Token ProxyAuth utilisé</span>
                    </div>
                  </div>
                </div>
              </template>
            </MazCard>

            <!-- Kubeconfig -->
            <MazCard class="kubeconfig-card full-width">
              <template #content-title>
                <h3 class="card-title">
                  <MazIcon :icon="LazyMazCloudArrowDown" size="lg" />
                  Configuration Kubernetes
                </h3>
              </template>
              <template #default>
                <div class="kubeconfig-content">
                  <p class="kubeconfig-description">
                    Téléchargez la configuration Kubernetes pour accéder au cluster. Le kubeconfig utilise
                    le proxy ProxyAuth comme endpoint et votre token d'authentification. Ce token est valide
                    tant que votre session ProxyAuth est active.
                  </p>

                  <div class="kubeconfig-field" v-highlight>
                    <pre><code class="hljs yaml">{{ generateKubeconfig() }}</code></pre>
                  </div>

                  <div class="kubeconfig-actions">
                    <MazBtn color="success" size="lg" :left-icon="LazyMazCloudArrowDown" @click="downloadKubeconfig"
                      class="download-button">
                      Télécharger kubeconfig
                    </MazBtn>
                    <MazBtn color="primary" size="lg" :left-icon="LazyMazClipboardDocument"
                      @click="copyToClipboard(generateKubeconfig(), 'Kubeconfig')" class="copy-kubeconfig-button">
                      Copier kubeconfig
                    </MazBtn>
                  </div>
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
                <MazIcon :icon="LazyMazArrowTopRightOnSquare" size="xl" class="error-icon" />
                <h2 class="error-title">Cluster introuvable</h2>
                <p class="error-description">
                  Le cluster demandé est introuvable ou inaccessible.
                  Veuillez vérifier l'URL ou contacter votre administrateur.
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
.cluster-nosso-container {
  min-height: 100vh;
  background: linear-gradient(135deg, #0f172a 0%, #1e293b 100%);
  font-family: var(--font-family-sans, 'Inter', sans-serif);
}

/* Header Section */
.header-section {
  background: linear-gradient(135deg, #0891b2 0%, #06b6d4 100%);
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
    radial-gradient(circle at 20% 80%, rgba(6, 182, 212, 0.4) 0%, transparent 50%),
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

/* Loading Step */
.loading-card {
  background: #1e293b;
  border: 1px solid rgba(255, 255, 255, 0.1);
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.3);
}

.loading-content {
  text-align: center;
  padding: 3rem 2rem;
}

.loading-spinner {
  margin-bottom: 2rem;
}

.loading-title {
  font-size: 1.5rem;
  font-weight: 600;
  color: #f1f5f9;
  margin: 0 0 1rem 0;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 0.5rem;
}

.loading-description {
  font-size: 1.125rem;
  color: #94a3b8;
  margin: 0;
  line-height: 1.6;
}

/* Cluster Step */
.cluster-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(400px, 1fr));
  gap: 1.5rem;
}

.full-width {
  grid-column: 1 / -1;
}

.info-card,
.auth-card,
.info-box,
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

/* Info Box */
.info-box {
  background: rgba(6, 182, 212, 0.1);
  border-left: 4px solid #06b6d4;
}

.info-box-content {
  padding: 1rem;
}

.info-message {
  color: #cbd5e1;
  line-height: 1.6;
  margin: 0;
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

/* Auth Card */
.auth-content {
  padding: 1rem 0;
}

.auth-description {
  color: #94a3b8;
  margin: 0 0 1.5rem 0;
  line-height: 1.6;
}

.auth-details {
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
}

.auth-item {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  color: #cbd5e1;
  padding: 0.5rem;
  background: rgba(255, 255, 255, 0.05);
  border-radius: 6px;
}

.auth-icon {
  color: #06b6d4;
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

.kubeconfig-field {
  margin-bottom: 1.5rem;
  border-radius: 8px;
  overflow: hidden;
  background: #282c34;
  border: 1px solid rgba(255, 255, 255, 0.1);
  max-height: 400px;
  overflow-y: auto;
}

.kubeconfig-field pre {
  margin: 0;
  padding: 1.5rem;
  background: transparent;
  overflow-x: auto;
  line-height: 1.5;
}

.kubeconfig-field pre code {
  font-family: 'Monaco', 'Menlo', 'Ubuntu Mono', monospace;
  font-size: 0.75rem;
  background: transparent;
  color: #abb2bf;
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

/* Responsive Design */
@media (max-width: 768px) {
  .page-title {
    font-size: 1.75rem;
    flex-direction: column;
    gap: 0.5rem;
  }

  .cluster-grid {
    grid-template-columns: 1fr;
  }

  .kubeconfig-actions {
    flex-direction: column;
  }

  .download-button,
  .copy-kubeconfig-button {
    width: 100%;
  }
}

@media (max-width: 480px) {
  .main-section {
    padding: 2rem 1rem;
  }

  .header-section {
    padding: 1.5rem 1rem 1rem;
  }

  .loading-content {
    padding: 2rem 1rem;
  }
}
</style>
