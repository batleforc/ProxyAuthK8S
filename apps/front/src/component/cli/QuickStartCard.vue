<script setup lang="ts">
import MazBadge from 'maz-ui/components/MazBadge';
import { LazyMazPlay as MazPlay } from '@maz-ui/icons';
import SectionCard from '../common/SectionCard.vue';

const steps = [
  'Installer le plugin via Krew ou manuellement',
  'Configurer le serveur ProxyAuthK8s',
  'S\'authentifier et utiliser kubectl normalement',
];
</script>

<template>
  <SectionCard
    class="quick-start-card"
    heading-tag="h2"
    title="Démarrage Rapide"
    :icon="MazPlay"
  >
    <div class="quick-start-content">
      <div class="quick-steps">
        <div
          v-for="(step, index) in steps"
          :key="step"
          class="step"
        >
          <MazBadge
            color="primary"
            size="sm"
          >
            {{ index + 1 }}
          </MazBadge>
          <span>{{ step }}</span>
        </div>
      </div>
    </div>
  </SectionCard>
</template>

<style scoped>
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

@media (max-width: 768px) {
  .quick-steps {
    gap: 0.75rem;
  }

  .step {
    font-size: 1rem;
  }
}
</style>
