<script setup lang="ts">
import { computed } from 'vue';
import 'highlight.js/styles/atom-one-dark.css';
import MazIcon from 'maz-ui/components/MazIcon';
import { LazyMazCommandLine as MazCommandLine } from '@maz-ui/icons';
import { useAuthStore } from '../store/auth';
import QuickStartCard from '../component/cli/QuickStartCard.vue';
import InstallationCard from '../component/cli/InstallationCard.vue';
import UsageGuideCard from '../component/cli/UsageGuideCard.vue';
import WorkflowCard from '../component/cli/WorkflowCard.vue';
import ResourcesCard from '../component/cli/ResourcesCard.vue';
import {
  backendUrlToName,
  buildUsageExamples,
  buildWorkflowExample,
  resolveBackendUrl,
} from '../utils/cliCommands.ts';

const authStore = useAuthStore();

const backendUrl = computed(() => {
  const envUrl = import.meta.env.VITE_API_BASE_URL;
  return resolveBackendUrl(envUrl, globalThis.location.origin);
});

const backendUrlName = computed(() => {
  try {
    return backendUrlToName(backendUrl.value);
  } catch (error) {
    console.error("Error parsing backend URL:", error);
    return backendUrl.value;
  }
});

// Built once at setup (not reactive), like the original inline definitions.
const usageExamples = buildUsageExamples(backendUrl.value, backendUrlName.value);
const workflowExample = buildWorkflowExample(backendUrl.value, backendUrlName.value, authStore.getToken);
</script>

<template>
  <div class="cli-view-container">
    <!-- Header Section -->
    <section class="header-section">
      <div class="header-content">
        <h1 class="page-title">
          <MazIcon
            :icon="MazCommandLine"
            size="xl"
            class="title-icon"
          />
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
        <QuickStartCard />
        <InstallationCard />
        <UsageGuideCard :usage-examples="usageExamples" />
        <WorkflowCard :workflow-example="workflowExample" />
        <ResourcesCard />
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

/* Responsive Design */
@media (max-width: 768px) {
  .page-title {
    font-size: 2rem;
    flex-direction: column;
    gap: 0.5rem;
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
}
</style>
