<script setup lang="ts" generic="T extends string">
import { useId } from 'vue';

/**
 * A three-state segmented group (DESIGN.md "Preferences"): one 1px frame, 1px
 * dividers, the chosen cell reversed to an ink block, state on aria-pressed.
 * The note appears only while the preference is System.
 */
defineProps<{
  label: string;
  options: { value: T; label: string }[];
  note: string;
}>();
const model = defineModel<T>({ required: true });
const id = useId();
</script>

<template>
  <div>
    <p
      :id="id"
      class="mb-2 mt-4 text-xs uppercase tracking-[0.14em] text-muted-foreground first:mt-0"
    >
      {{ label }}
    </p>
    <div
      role="group"
      :aria-labelledby="id"
      class="flex border border-border"
    >
      <button
        v-for="option in options"
        :key="option.value"
        type="button"
        :aria-pressed="model === option.value"
        class="flex-1 cursor-pointer border-r border-border bg-transparent px-1 py-2 text-xs uppercase tracking-[0.06em] text-muted-foreground last:border-r-0 hover:text-foreground aria-pressed:bg-foreground aria-pressed:font-bold aria-pressed:text-background"
        @click="model = option.value"
      >
        {{ option.label }}
      </button>
    </div>
    <p
      v-if="model === 'system'"
      class="mt-2 text-xs text-muted-foreground"
    >
      {{ note }}
    </p>
  </div>
</template>
