<script lang="ts" setup>
import { computed, onMounted } from 'vue';
import { useClustersStore } from '../store/clusters.ts';
import { useAuthStore } from '../store/auth.ts';
import MazCard from 'maz-ui/components/MazCard';
import MazBtn from 'maz-ui/components/MazBtn';
import MazIcon from 'maz-ui/components/MazIcon';
import MazBadge from 'maz-ui/components/MazBadge';
import MazSpinner from 'maz-ui/components/MazSpinner';
import {
  MazServer,
  MazShieldCheck,
  MazExclamationTriangle,
  MazCheckCircle,
  MazXCircle,
  MazArrowTopRightOnSquare,
  MazCog6Tooth
} from '@maz-ui/icons';
import { useToast } from 'maz-ui/composables/useToast';
import { VisibleCluster } from '@proxy-auth-k8s/front-api';

const clustersStore = useClustersStore();
const authStore = useAuthStore();
const toast = useToast();

onMounted(() => {
  if (authStore.inited && authStore.user && clustersStore.inited === false) {
    clustersStore.fetchClusters(toast);
  }
});
const clusters = computed(() => clustersStore.getClusters);
const isLoading = computed(() => !clustersStore.isInited);

const getClusterStatusColor = (cluster: VisibleCluster) => {
  if (!cluster.enabled) return 'danger';
  if (cluster.is_reachable === false) return 'warning';
  if (cluster.is_reachable === true) return 'success';
  return 'info';
};

const getClusterStatusText = (cluster: VisibleCluster) => {
  if (!cluster.enabled) return 'Désactivé';
  if (cluster.is_reachable === false) return 'Injoignable';
  if (cluster.is_reachable === true) return 'Disponible';
  return 'État inconnu';
};

const getClusterStatusIcon = (cluster: VisibleCluster) => {
  if (!cluster.enabled) return MazXCircle;
  if (cluster.is_reachable === false) return MazExclamationTriangle;
  if (cluster.is_reachable === true) return MazCheckCircle;
  return MazCog6Tooth;
};

const handleClusterAccess = (cluster: VisibleCluster) => {
  // TODO: Implement cluster access logic
  console.log('Accessing cluster:', cluster.name);
  toast.warning(`Accès au cluster ${cluster.name} (fonctionnalité à implémenter)`);
  clustersStore.redirectToLogin(cluster.namespace, cluster.name);
};

const getClusterURL = (cluster: VisibleCluster) => {
  return `${window.location.origin}/clusters/${cluster.namespace}/${cluster.name}`;
};

const copyUrlToClipboard = async (cluster: VisibleCluster) => {
  try {
    await navigator.clipboard.writeText(getClusterURL(cluster));
    toast.info("URL du cluster copiée dans le presse-papiers !");
  } catch (error) {
    toast.error("Échec de la copie de l\'URL du cluster.");
  }
}
</script>

<template>
  <div class="home-logged-container">
    <!-- Header Section -->
    <section class="header-section">
      <div class="header-content">
        <h1 class="welcome-title">
          Bonjour {{ authStore.user?.profile?.name || 'Utilisateur' }} !
        </h1>
        <p class="welcome-description">
          Accédez à vos clusters Kubernetes en toute sécurité. Sélectionnez le cluster souhaité pour commencer.
        </p>
      </div>
    </section>

    <!-- Clusters Section -->
    <section class="clusters-section">
      <div class="clusters-container">
        <div class="section-header">
          <h2 class="section-title">
            <MazIcon :icon="MazServer" size="lg" class="title-icon" />
            Vos Clusters Kubernetes
          </h2>
          <p class="section-subtitle">
            {{ clusters.length }} cluster{{ clusters.length !== 1 ? 's' : '' }} disponible{{ clusters.length !== 1 ? 's'
              : '' }}
          </p>
        </div>

        <!-- Loading State -->
        <div v-if="isLoading" class="loading-container">
          <MazSpinner size="4em" color="primary" />
          <p class="loading-text">Chargement de vos clusters...</p>
        </div>

        <!-- Empty State -->
        <div v-else-if="clusters.length === 0" class="empty-state">
          <MazIcon :icon="MazServer" size="xl" class="empty-icon" />
          <h3 class="empty-title">Aucun cluster disponible</h3>
          <p class="empty-description">
            Vous n'avez actuellement accès à aucun cluster Kubernetes.
            Contactez votre administrateur pour obtenir les permissions nécessaires.
          </p>
        </div>

        <!-- Clusters Grid -->
        <div v-else class="clusters-grid">
          <MazCard v-for="cluster in clusters" :key="`${cluster.namespace}-${cluster.name}`" class="cluster-card"
            :class="{
              'cluster-disabled': !cluster.enabled,
              'cluster-unreachable': cluster.is_reachable === false
            }">
            <template #content-title>
              <div class="cluster-header">
                <div class="cluster-info">
                  <h3 class="cluster-name">{{ cluster.name }}</h3>
                  <p class="cluster-namespace">Namespace: {{ cluster.namespace }}</p>
                </div>
                <div class="cluster-status">
                  <MazBadge :color="getClusterStatusColor(cluster)" size="sm">
                    <MazIcon :icon="getClusterStatusIcon(cluster)" size="xs" class="status-icon" />
                    {{ getClusterStatusText(cluster) }}
                  </MazBadge>
                </div>
              </div>
            </template>

            <template #default>
              <div class="cluster-features">
                <div class="feature-item">
                  <MazIcon :icon="MazShieldCheck" size="sm"
                    :class="cluster.sso_enabled ? 'feature-enabled' : 'feature-disabled'" />
                  <span class="feature-text">
                    SSO {{ cluster.sso_enabled ? 'activé' : 'désactivé' }}
                  </span>
                </div>
                <div class="feature-item cluster-url" @click="copyUrlToClipboard(cluster)" style="cursor: pointer;">
                  <MazIcon :icon="MazArrowTopRightOnSquare" size="sm" class="url-icon" />
                  <span class="url-text">
                    {{ getClusterURL(cluster) }}
                  </span>
                </div>
              </div>
            </template>

            <template #footer>
              <div class="cluster-actions">
                <MazBtn :disabled="!cluster.enabled || cluster.is_reachable === false" color="primary" size="sm"
                  @click="handleClusterAccess(cluster)" :right-icon="MazArrowTopRightOnSquare" class="access-button">
                  Accéder au cluster
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
.home-logged-container {
  min-height: 100vh;
  background: linear-gradient(135deg, #0f172a 0%, #1e293b 100%);
  font-family: var(--font-family-sans, 'Inter', sans-serif);
}

/* Header Section */
.header-section {
  background: linear-gradient(135deg, #1e40af 0%, #7c3aed 100%);
  color: white;
  padding: 3rem 2rem 2rem;
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
}

.welcome-title {
  font-size: 2.5rem;
  font-weight: 700;
  margin: 0 0 1rem 0;
  letter-spacing: -0.025em;
}

.welcome-description {
  font-size: 1.125rem;
  color: rgba(255, 255, 255, 0.9);
  margin: 0;
  max-width: 600px;
}

/* Clusters Section */
.clusters-section {
  padding: 3rem 2rem;
}

.clusters-container {
  max-width: 1200px;
  margin: 0 auto;
}

.section-header {
  text-align: center;
  margin-bottom: 3rem;
}

.section-title {
  font-size: 2rem;
  font-weight: 700;
  color: #f1f5f9;
  margin: 0 0 0.5rem 0;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 0.75rem;
}

.title-icon {
  color: #a78bfa;
}

.section-subtitle {
  font-size: 1.125rem;
  color: #94a3b8;
  margin: 0;
}

/* Loading State */
.loading-container {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 1rem;
  padding: 4rem 2rem;
}

.loading-text {
  font-size: 1.125rem;
  color: #94a3b8;
  margin: 0;
}

/* Empty State */
.empty-state {
  text-align: center;
  padding: 4rem 2rem;
}

.empty-icon {
  color: #64748b;
  margin-bottom: 1.5rem;
}

.empty-title {
  font-size: 1.5rem;
  font-weight: 600;
  color: #cbd5e1;
  margin: 0 0 1rem 0;
}

.empty-description {
  font-size: 1.125rem;
  color: #94a3b8;
  margin: 0;
  max-width: 500px;
  margin-left: auto;
  margin-right: auto;
  line-height: 1.6;
}

/* Clusters Grid */
.clusters-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(350px, 1fr));
  gap: 1.5rem;
}

.cluster-card {
  transition: all 0.3s ease;
  border: 1px solid rgba(255, 255, 255, 0.1);
  background: #1e293b;
  box-shadow: 0 4px 6px rgba(0, 0, 0, 0.3);
}

.cluster-card:hover {
  transform: translateY(-4px);
  box-shadow: 0 20px 40px rgba(0, 0, 0, 0.4);
  border-color: rgba(167, 139, 250, 0.3);
}

.cluster-card.cluster-disabled {
  opacity: 0.7;
  border-color: #ef4444;
  background: #2d1b1b;
}

.cluster-card.cluster-unreachable {
  border-color: #f59e0b;
  background: #2d2317;
}

/* Cluster Header */
.cluster-header {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  gap: 1rem;
}

.cluster-info {
  flex: 1;
}

.cluster-name {
  font-size: 1.25rem;
  font-weight: 600;
  color: #f1f5f9;
  margin: 0 0 0.25rem 0;
}

.cluster-namespace {
  font-size: 0.875rem;
  color: #94a3b8;
  margin: 0;
}

.cluster-status {
  flex-shrink: 0;
}

.status-icon {
  margin-right: 0.25rem;
}

/* Cluster Features */
.cluster-features {
  padding: 1rem 0;
}

.feature-item {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  margin-bottom: 0.5rem;
}

.feature-enabled {
  color: #34d399;
}

.feature-disabled {
  color: #94a3b8;
}

.feature-text {
  font-size: 0.875rem;
  color: #cbd5e1;
}

/* Cluster URL */
.cluster-url {
  background: rgba(167, 139, 250, 0.1);
  padding: 0.75rem;
  border-radius: 8px;
  border: 1px solid rgba(167, 139, 250, 0.2);
  margin-top: 0.75rem;
  transition: all 0.3s ease;
}

.cluster-url:hover {
  background: rgba(167, 139, 250, 0.15);
  border-color: rgba(167, 139, 250, 0.3);
}

.url-icon {
  color: #a78bfa;
  flex-shrink: 0;
}

.url-text {
  font-size: 0.8rem;
  color: #e2e8f0;
  font-family: 'Monaco', 'Menlo', 'Ubuntu Mono', monospace;
  font-weight: 500;
  word-break: break-all;
  line-height: 1.4;
}

/* Cluster Actions */
.cluster-actions {
  display: flex;
  justify-content: flex-end;
}

.access-button {
  min-width: 140px;
}

/* Responsive Design */
@media (max-width: 768px) {
  .welcome-title {
    font-size: 2rem;
  }

  .section-title {
    font-size: 1.75rem;
    flex-direction: column;
    gap: 0.5rem;
  }

  .clusters-grid {
    grid-template-columns: 1fr;
  }

  .cluster-header {
    flex-direction: column;
    align-items: flex-start;
    gap: 0.75rem;
  }

  .cluster-actions {
    justify-content: stretch;
  }

  .access-button {
    width: 100%;
  }
}

@media (max-width: 480px) {
  .header-section {
    padding: 2rem 1rem 1.5rem;
  }

  .clusters-section {
    padding: 2rem 1rem;
  }

  .welcome-title {
    font-size: 1.75rem;
  }
}
</style>