<script setup lang="ts">
/** "Label: value" row of the cluster information cards. Pass `value` for monospace text, or a slot (e.g. a badge). */
defineProps<{
  label: string;
  value?: string;
}>();
</script>

<template>
  <div class="info-row">
    <span class="info-label">{{ label }}</span>
    <span
      v-if="value !== undefined"
      class="info-value"
    >{{ value }}</span>
    <slot v-else />
  </div>
</template>

<style scoped>
.info-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 1rem;
  padding: 0.75rem;
  background: rgba(255, 255, 255, 0.05);
  border-radius: 8px;
}

.info-label {
  font-weight: 500;
  color: #cbd5e1;
}

.info-value {
  color: #f1f5f9;
  font-family: 'Monaco', 'Menlo', 'Ubuntu Mono', monospace;
  font-size: 0.875rem;
  word-break: break-all;
}
</style>
