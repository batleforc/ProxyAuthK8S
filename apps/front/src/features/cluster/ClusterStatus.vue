<script setup lang="ts">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import { Resolution, type ResolutionState } from '@/components/ui/resolution';
import type { ClusterStatusKind } from './status.ts';

/**
 * A cluster status on three channels (RFC 0002 §4.2): pattern, word, hue.
 *
 * | status      | pattern            | hue     |
 * | available   | fine 3×3, all lit  | ink     |
 * | unreachable | coarse 2×2, all    | accent  |
 * | unknown     | coarse 2×2, 3 of 4 | copper (never "healthy") |
 * | disabled    | none, dotted rule  | ink-dim |
 */
const props = defineProps<{ status: ClusterStatusKind }>();
const { t } = useI18n();

const RESOLUTION: Partial<Record<ClusterStatusKind, ResolutionState>> = {
  available: 'cached',
  unreachable: 'blocked',
  unknown: 'held',
};
const state = computed(() => RESOLUTION[props.status]);
const label = computed(() => t(`cluster.status.${props.status}`));
</script>

<template>
  <Resolution
    v-if="state"
    :state="state"
    :label="label"
    :data-status="status"
  />
  <span
    v-else
    class="inline-block border-b border-dashed border-rule-soft font-mono text-xs uppercase tracking-[0.1em] text-muted-foreground"
    :data-status="status"
    >{{ label }}</span
  >
</template>
