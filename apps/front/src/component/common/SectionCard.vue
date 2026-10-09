<script setup lang="ts">
import MazCard from 'maz-ui/components/MazCard';
import MazIcon from 'maz-ui/components/MazIcon';
import type { IconComponent } from '@maz-ui/icons';

/**
 * Dark MazCard used by the CLI / cluster pages, with an optional icon + title heading.
 * Extra classes (e.g. `full-width`) fall through to the MazCard root.
 */
withDefaults(
  defineProps<{
    title?: string;
    icon?: IconComponent;
    headingTag?: 'h2' | 'h3';
  }>(),
  { title: undefined, icon: undefined, headingTag: 'h3' },
);
</script>

<template>
  <MazCard class="section-card">
    <template
      v-if="title"
      #content-title
    >
      <component
        :is="headingTag"
        class="card-title"
        :class="`card-title--${headingTag}`"
      >
        <MazIcon
          :icon="icon"
          size="lg"
        />
        {{ title }}
      </component>
    </template>
    <template #default>
      <slot />
    </template>
  </MazCard>
</template>

<style scoped>
.section-card {
  background: #1e293b;
  border: 1px solid rgba(255, 255, 255, 0.1);
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.3);
}

.card-title {
  font-weight: 600;
  color: #f1f5f9;
  margin: 0;
  display: flex;
  align-items: center;
}

/* Page-level cards (CLI guide) */
.card-title--h2 {
  font-size: 1.5rem;
  gap: 0.75rem;
}

/* Grid cards (cluster pages) */
.card-title--h3 {
  font-size: 1.25rem;
  gap: 0.5rem;
}
</style>
