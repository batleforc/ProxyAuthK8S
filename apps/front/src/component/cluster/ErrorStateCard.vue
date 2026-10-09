<script setup lang="ts">
import MazCard from 'maz-ui/components/MazCard';
import MazBtn from 'maz-ui/components/MazBtn';
import MazIcon from 'maz-ui/components/MazIcon';
import { LazyMazArrowTopRightOnSquare } from '@maz-ui/icons';

/** Red-bordered failure card with a "back to home" button. */
defineProps<{
  title: string;
  description: string;
}>();
</script>

<template>
  <MazCard class="error-card">
    <template #default>
      <div class="error-content">
        <MazIcon
          :icon="LazyMazArrowTopRightOnSquare"
          size="xl"
          class="error-icon"
        />
        <h2 class="error-title">
          {{ title }}
        </h2>
        <p class="error-description">
          {{ description }}
        </p>
        <MazBtn
          color="primary"
          size="lg"
          class="back-button"
          @click="$router.push({ name: 'home' })"
        >
          Retour à l'accueil
        </MazBtn>
      </div>
    </template>
  </MazCard>
</template>

<style scoped>
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
</style>
