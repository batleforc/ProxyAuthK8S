<script setup lang="ts">
import { computed, onMounted, ref } from 'vue';
import { useClustersStore } from '../store/clusters.ts';
import { useAuthStore } from '../store/auth.ts';
import { useToast } from 'maz-ui/composables/useToast';
import { useRoute, useRouter } from 'vue-router';
import type { VisibleCluster } from '@proxy-auth-k8s/front-api';
import MazIcon from 'maz-ui/components/MazIcon';
import { LazyMazServer } from '@maz-ui/icons';
import LoadingStateCard from '../component/cluster/LoadingStateCard.vue';
import ErrorStateCard from '../component/cluster/ErrorStateCard.vue';
import NoSsoInfoBox from '../component/nosso/NoSsoInfoBox.vue';
import NoSsoClusterInfoCard from '../component/nosso/NoSsoClusterInfoCard.vue';
import NoSsoAuthCard from '../component/nosso/NoSsoAuthCard.vue';
import NoSsoKubeconfigCard from '../component/nosso/NoSsoKubeconfigCard.vue';

const clustersStore = useClustersStore();
const authStore = useAuthStore();
const toast = useToast();
const route = useRoute();
const router = useRouter();

// État de chargement
const isLoading = ref(true);
const clusterData = ref<VisibleCluster | null>(null);

// Lifecycle
onMounted(async () => {
  const ns = route.params.ns as string;
  const cluster = route.params.cluster as string;

  if (!ns || !cluster) {
    toast.error('Paramètres manquants dans l\'URL', { timeout: 5000 });
    setTimeout(() => {
      router.push({ name: 'home' });
    }, 2000);
    return;
  }

  // Vérifier si les clusters sont chargés
  if (!clustersStore.isInited) {
    toast.info('Chargement des clusters...', { timeout: 3000 });
    await clustersStore.fetchClusters(toast);
  }

  // Trouver le cluster
  const foundCluster = clustersStore.getClusters.find(
    c => c.namespace === ns && c.name === cluster
  );

  if (!foundCluster) {
    toast.error('Cluster introuvable', { timeout: 5000 });
    setTimeout(() => {
      router.push({ name: 'home' });
    }, 2000);
    return;
  }

  if (foundCluster.sso_enabled) {
    toast.warning('Ce cluster utilise SSO, redirection vers la page appropriée...', { timeout: 3000 });
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
</script>

<template>
  <div class="cluster-nosso-container">
    <!-- Header Section -->
    <section class="header-section">
      <div class="header-content">
        <h1 class="page-title">
          <MazIcon
            :icon="LazyMazServer"
            size="lg"
            class="title-icon"
          />
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
        <div
          v-if="isLoading"
          class="loading-step"
        >
          <LoadingStateCard
            title="Chargement des informations du cluster..."
            description="Récupération des informations du cluster. Veuillez patienter."
          />
        </div>

        <!-- Affichage du cluster -->
        <div
          v-else-if="clusterData"
          class="cluster-step"
        >
          <div class="cluster-grid">
            <NoSsoInfoBox class="full-width" />
            <NoSsoClusterInfoCard
              :cluster="clusterData"
              :cluster-url="clusterUrl"
              :username="authStore.user?.profile?.preferred_username || 'N/A'"
            />
            <NoSsoAuthCard />
            <NoSsoKubeconfigCard
              class="full-width"
              :cluster="clusterData"
              :cluster-url="clusterUrl"
              :username="authStore.user?.profile?.preferred_username || 'user'"
            />
          </div>
        </div>

        <!-- État d'erreur -->
        <div
          v-else
          class="error-step"
        >
          <ErrorStateCard
            title="Cluster introuvable"
            description="Le cluster demandé est introuvable ou inaccessible. Veuillez vérifier l'URL ou contacter votre administrateur."
          />
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

/* Cluster Step */
.cluster-grid {
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

  .cluster-grid {
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
