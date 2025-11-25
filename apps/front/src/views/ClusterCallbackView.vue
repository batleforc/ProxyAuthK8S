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
import {
  MazServer,
  MazShieldCheck,
  MazCheckCircle,
  MazArrowTopRightOnSquare,
  MazClipboardDocument,
  MazCloudArrowDown,
  MazKey,
  MazUser,
  MazGlobeAlt,
  MazClock
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

// Actions
const copyToClipboard = async (text: string, label: string) => {
  try {
    await navigator.clipboard.writeText(text);
    toast.success(`${label} copié dans le presse-papiers !`);
  } catch (error) {
    toast.error(`Échec de la copie du ${label.toLowerCase()}.`);
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
  document.body.removeChild(a);
  URL.revokeObjectURL(url);
  toast.success('Kubeconfig téléchargé avec succès !');
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
                    <label class="token-label">Access Token:</label>
                    <div class="token-field">
                      <MazTextarea :model-value="callbackData.retour.access_token" readonly :rows="3"
                        class="token-textarea" />
                      <MazBtn size="sm" color="primary" :left-icon="MazClipboardDocument"
                        @click="copyToClipboard(callbackData.retour.access_token, 'Access Token')" class="copy-button">
                        Copier
                      </MazBtn>
                    </div>
                  </div>

                  <div class="token-section">
                    <label class="token-label">ID Token:</label>
                    <div class="token-field">
                      <MazTextarea :model-value="callbackData.retour.id_token" readonly :rows="3"
                        class="token-textarea" />
                      <MazBtn size="sm" color="primary" :left-icon="MazClipboardDocument"
                        @click="copyToClipboard(callbackData.retour.id_token, 'ID Token')" class="copy-button">
                        Copier
                      </MazBtn>
                    </div>
                  </div>

                  <div class="token-section">
                    <label class="token-label">Refresh Token:</label>
                    <div class="token-field">
                      <MazTextarea :model-value="callbackData.retour.refresh_token" readonly :rows="3"
                        class="token-textarea" />
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
                    Utilisez ce fichier kubeconfig pour vous connecter à votre cluster Kubernetes avec kubectl.
                  </p>

                  <div class="kubeconfig-field">
                    <MazTextarea :model-value="generateKubeconfig()" readonly :rows="15" class="kubeconfig-textarea" />
                  </div>

                  <div class="kubeconfig-actions">
                    <MazBtn color="success" size="lg" :left-icon="MazCloudArrowDown" @click="downloadKubeconfig"
                      class="download-button">
                      Télécharger kubeconfig
                    </MazBtn>
                    <MazBtn color="primary" size="lg" :left-icon="MazClipboardDocument"
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

.kubeconfig-field {
  margin-bottom: 1.5rem;
}

.kubeconfig-textarea {
  font-family: 'Monaco', 'Menlo', 'Ubuntu Mono', monospace;
  font-size: 0.75rem;
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