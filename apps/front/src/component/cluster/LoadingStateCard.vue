<script setup lang="ts">
import MazIcon from 'maz-ui/components/MazIcon';
import MazSpinner from 'maz-ui/components/MazSpinner';
import { LazyMazClock, type IconComponent } from '@maz-ui/icons';
import SectionCard from '../common/SectionCard.vue';

/** Spinner card shown while a cluster page waits for data. */
withDefaults(
  defineProps<{
    title: string;
    description: string;
    /** Optional icon + text facts displayed under the description. */
    details?: { icon: IconComponent; text: string }[];
  }>(),
  { details: () => [] },
);
</script>

<template>
  <SectionCard>
    <div class="loading-content">
      <MazSpinner
        size="3rem"
        color="primary"
        class="loading-spinner"
      />
      <h2 class="loading-title">
        <MazIcon
          :icon="LazyMazClock"
          size="lg"
        />
        {{ title }}
      </h2>
      <p
        class="loading-description"
        :class="{ 'loading-description--spaced': details.length > 0 }"
      >
        {{ description }}
      </p>
      <div
        v-if="details.length > 0"
        class="loading-details"
      >
        <div
          v-for="detail in details"
          :key="detail.text"
          class="detail-item"
        >
          <MazIcon
            :icon="detail.icon"
            size="sm"
            class="detail-icon"
          />
          <span>{{ detail.text }}</span>
        </div>
      </div>
    </div>
  </SectionCard>
</template>

<style scoped>
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

.loading-description--spaced {
  margin: 0 0 2rem 0;
}

.loading-details {
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

@media (max-width: 768px) {
  .loading-details {
    flex-direction: column;
    gap: 1rem;
  }
}

@media (max-width: 480px) {
  .loading-content {
    padding: 2rem 1rem;
  }
}
</style>
