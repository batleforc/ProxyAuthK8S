<script setup lang="ts">
import MazBtn from 'maz-ui/components/MazBtn';
import MazIcon from 'maz-ui/components/MazIcon';
import {
  LazyMazCloudArrowDown as MazCloudArrowDown,
  LazyMazCommandLine as MazCommandLine,
  LazyMazShieldCheck as MazShieldCheck,
  LazyMazDocumentText as MazDocumentText,
  LazyMazArrowTopRightOnSquare as MazArrowTopRightOnSquare,
} from '@maz-ui/icons';
import SectionCard from '../common/SectionCard.vue';

// Helper function for external links
const openExternalLink = (url: string) => {
  window.open(url, '_blank');
};

const resources = [
  {
    icon: MazShieldCheck,
    title: 'Documentation Complète',
    description: 'Consultez le README du plugin pour plus de détails sur toutes les fonctionnalités.',
    color: 'primary',
    action: 'Voir le README',
    url: 'https://github.com/batleforc/proxyauthk8s/tree/main/apps/kubectl_proxyauth',
  },
  {
    icon: MazCloudArrowDown,
    title: 'Releases GitHub',
    description: 'Téléchargez les dernières versions du plugin pour tous les OS.',
    color: 'success',
    action: 'Voir les releases',
    url: 'https://github.com/batleforc/proxyauthk8s/releases',
  },
  {
    icon: MazCommandLine,
    title: 'Index Krew',
    description: 'Plugin disponible dans l\'index officiel Krew pour une installation facile.',
    color: 'info',
    action: 'Voir Krew',
    url: 'https://krew.sigs.k8s.io/plugins/',
  },
] as const;
</script>

<template>
  <SectionCard
    class="resources-card"
    heading-tag="h2"
    title="Ressources Supplémentaires"
    :icon="MazDocumentText"
  >
    <div class="resources-content">
      <div class="resources-grid">
        <div
          v-for="resource in resources"
          :key="resource.title"
          class="resource-item"
        >
          <MazIcon
            :icon="resource.icon"
            size="lg"
            class="resource-icon"
          />
          <h3 class="resource-title">
            {{ resource.title }}
          </h3>
          <p class="resource-description">
            {{ resource.description }}
          </p>
          <MazBtn
            :color="resource.color"
            size="sm"
            :right-icon="MazArrowTopRightOnSquare"
            @click="openExternalLink(resource.url)"
          >
            {{ resource.action }}
          </MazBtn>
        </div>
      </div>
    </div>
  </SectionCard>
</template>

<style scoped>
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

@media (max-width: 768px) {
  .resources-grid {
    grid-template-columns: 1fr;
  }
}
</style>
