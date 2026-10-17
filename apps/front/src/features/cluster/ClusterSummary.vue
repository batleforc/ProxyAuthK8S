<script setup lang="ts">
import { useI18n } from 'vue-i18n';
import ClusterStatus from './ClusterStatus.vue';
import type { ClusterStatusKind } from './status.ts';

/**
 * The facts of one cluster as ruled rows (no card). Shared by the SSO
 * callback (API URL) and the no-SSO page (proxy URL): `urlLabel` says which.
 */
defineProps<{
  name: string;
  namespace: string;
  url: string;
  urlLabel: string;
  user: string;
  status?: ClusterStatusKind;
}>();
const { t } = useI18n();
</script>

<template>
  <dl class="m-0 grid grid-cols-1 sm:grid-cols-[minmax(0,14rem)_minmax(0,1fr)]">
    <template
      v-for="row in [
        { term: t('cluster.name'), value: name },
        { term: t('cluster.namespace'), value: namespace },
        { term: urlLabel, value: url },
        { term: t('cluster.user'), value: user },
      ]"
      :key="row.term"
    >
      <dt
        class="border-b border-dashed border-rule-soft pb-1 pt-3 text-xs uppercase tracking-[0.14em] text-muted-foreground sm:pb-3"
      >
        {{ row.term }}
      </dt>
      <dd
        class="m-0 border-b border-dashed border-rule-soft pb-3 pt-1 text-base text-foreground [overflow-wrap:anywhere] sm:pt-3"
      >
        {{ row.value }}
      </dd>
    </template>
    <template v-if="status">
      <dt
        class="pb-1 pt-3 text-xs uppercase tracking-[0.14em] text-muted-foreground sm:pb-3"
      >
        {{ t('cluster.statusLabel') }}
      </dt>
      <dd class="m-0 pb-3 pt-1 sm:pt-3">
        <ClusterStatus :status="status" />
      </dd>
    </template>
  </dl>
</template>
