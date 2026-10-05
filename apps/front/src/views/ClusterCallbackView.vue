<script setup lang="ts">
import { computed, onMounted, ref } from 'vue';
import { useClustersStore } from '../store/clusters.ts';
import { useAuthStore } from '../store/auth.ts';
import { useToast } from 'maz-ui/composables/useToast';
import MazIcon from 'maz-ui/components/MazIcon';
import {
  LazyMazServer,
  LazyMazShieldCheck,
  LazyMazGlobeAlt,
} from '@maz-ui/icons';
import LoadingStateCard from '../component/cluster/LoadingStateCard.vue';
import ErrorStateCard from '../component/cluster/ErrorStateCard.vue';
import ClusterInfoCard from '../component/callback/ClusterInfoCard.vue';
import TokensCard from '../component/callback/TokensCard.vue';
import KubeconfigTabsCard from '../component/callback/KubeconfigTabsCard.vue';

const clustersStore = useClustersStore();
const authStore = useAuthStore();
const toast = useToast();

// État de l'authentification
const isAuthenticating = ref(true);
const authenticationComplete = ref(false);

// Lifecycle
onMounted(async () => {
  // Never log `authStore.user`: it holds the id/access/refresh tokens.
  if (authStore.isInited && authStore.isAuthenticated && clustersStore.inited === false) {
    toast.info('Récupération des clusters en cours...', { timeout: 3000 });
    await clustersStore.fetchClusters(toast);
  }
  if (authStore.isInited && authStore.isAuthenticated && clustersStore.inited) {
    try {
      toast.info('Finalisation de l\'authentification...', { timeout: 3000 });
      await clustersStore.callBackFromCluster(toast);
      authenticationComplete.value = true;
    } catch (error) {
      toast.error('Erreur lors de la finalisation de l\'authentification.', { timeout: 5000 });
      console.error('Error during callback:', error);
    } finally {
      isAuthenticating.value = false;
    }
  }
});


// Données calculées
const callbackData = computed(() => clustersStore.callBack);
const hasToken = computed(() => !!callbackData.value.retour?.access_token);
const currentCluster = computed(() =>
  clustersStore.getClusters.find(
    (cluster) =>
      cluster.namespace === callbackData.value.ns && cluster.name === callbackData.value.cluster
  )
);
const waitingDetails = computed(() => [
  { icon: LazyMazServer, text: `Cluster: ${callbackData.value.cluster || 'Chargement...'}` },
  { icon: LazyMazGlobeAlt, text: `Namespace: ${callbackData.value.ns || 'Chargement...'}` },
]);
</script>

<template>
  <div class="cluster-callback-container">
    <!-- Header Section -->
    <section class="header-section">
      <div class="header-content">
        <h1 class="page-title">
          <MazIcon
            :icon="LazyMazShieldCheck"
            size="lg"
            class="title-icon"
          />
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
        <div
          v-if="isAuthenticating"
          class="waiting-step"
        >
          <LoadingStateCard
            title="Récupération des tokens en cours..."
            description="Nous récupérons vos tokens d'authentification auprès du provider d'identité. Veuillez patienter quelques instants."
            :details="waitingDetails"
          />
        </div>

        <!-- Étape 2: Affichage des tokens et informations -->
        <div
          v-else-if="authenticationComplete && hasToken"
          class="success-step"
        >
          <div class="success-grid">
            <ClusterInfoCard
              :callback-data="callbackData"
              :current-cluster="currentCluster"
            />
            <TokensCard :tokens="callbackData.retour" />
            <KubeconfigTabsCard
              class="full-width"
              :callback-data="callbackData"
            />
          </div>
        </div>

        <!-- État d'erreur -->
        <div
          v-else
          class="error-step"
        >
          <ErrorStateCard
            title="Authentification échouée"
            description="Une erreur s'est produite lors de la récupération des tokens d'authentification. Veuillez réessayer ou contacter votre administrateur."
          />
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

/* Success Step */
.success-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(400px, 1fr));
  gap: 1.5rem;
}

.full-width {
  grid-column: 1 / -1;
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
}

@media (max-width: 480px) {
  .main-section {
    padding: 2rem 1rem;
  }

  .header-section {
    padding: 1.5rem 1rem 1rem;
  }
}
</style>
