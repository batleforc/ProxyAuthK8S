<script setup lang="ts">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import type { VisibleCluster } from '@proxy-auth-k8s/front-api';
import { TableCell, TableRow } from '@/components/ui/table';
import { Button } from '@/components/ui/button';
import ClusterStatus from './ClusterStatus.vue';
import { clusterStatus } from './status.ts';

/**
 * One cluster as a table row. Selecting it flips attributes in place (the
 * row is never rebuilt, so focus stays on the control): ink, a 1px crimson
 * edge and the raised fill, because no single channel is dependable.
 */
const props = defineProps<{ cluster: VisibleCluster; selected: boolean }>();
defineEmits<{ select: [] }>();
const { t } = useI18n();
const status = computed(() => clusterStatus(props.cluster));
</script>

<template>
  <TableRow :data-state="selected ? 'selected' : undefined">
    <TableCell
      class="border-l pl-3"
      :class="selected ? 'border-l-primary' : 'border-l-transparent'"
    >
      <ClusterStatus :status="status" />
    </TableCell>
    <TableCell class="text-base text-foreground [overflow-wrap:anywhere]">
      {{ cluster.name }}
    </TableCell>
    <TableCell class="text-muted-foreground [overflow-wrap:anywhere]">
      {{ cluster.namespace }}
    </TableCell>
    <TableCell
      class="hidden text-xs uppercase tracking-[0.1em] text-muted-foreground md:table-cell"
    >
      {{ cluster.sso_enabled ? t('cluster.sso.on') : t('cluster.sso.off') }}
    </TableCell>
    <TableCell class="text-right">
      <Button
        variant="outline"
        size="sm"
        :aria-pressed="selected"
        @click="$emit('select')"
      >
        {{ selected ? t('cluster.selected') : t('cluster.choose') }}
        <span class="sr-only">{{ cluster.name }}</span>
      </Button>
    </TableCell>
  </TableRow>
</template>
