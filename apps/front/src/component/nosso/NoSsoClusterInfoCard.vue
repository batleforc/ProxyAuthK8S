<script setup lang="ts">
import MazBadge from 'maz-ui/components/MazBadge';
import { LazyMazServer } from '@maz-ui/icons';
import type { VisibleCluster } from '@proxy-auth-k8s/front-api';
import SectionCard from '../common/SectionCard.vue';
import InfoRow from '../cluster/InfoRow.vue';

defineProps<{
  cluster: VisibleCluster;
  clusterUrl: string;
  username: string;
}>();
</script>

<template>
  <SectionCard
    class="info-card"
    title="Informations du Cluster"
    :icon="LazyMazServer"
  >
    <div class="info-content">
      <InfoRow label="Nom du cluster:">
        <MazBadge
          color="info"
          size="sm"
        >
          {{ cluster.name }}
        </MazBadge>
      </InfoRow>
      <InfoRow label="Namespace:">
        <MazBadge
          color="primary"
          size="sm"
        >
          {{ cluster.namespace }}
        </MazBadge>
      </InfoRow>
      <InfoRow
        label="URL du proxy:"
        :value="clusterUrl"
      />
      <InfoRow
        label="Utilisateur:"
        :value="username"
      />
      <InfoRow label="Statut:">
        <MazBadge
          :color="cluster.enabled ? 'success' : 'destructive'"
          size="sm"
        >
          {{ cluster.enabled ? 'Activé' : 'Désactivé' }}
        </MazBadge>
      </InfoRow>
      <InfoRow
        v-if="cluster.is_reachable !== null"
        label="Accessible:"
      >
        <MazBadge
          :color="cluster.is_reachable ? 'success' : 'warning'"
          size="sm"
        >
          {{ cluster.is_reachable ? 'Oui' : 'Non' }}
        </MazBadge>
      </InfoRow>
    </div>
  </SectionCard>
</template>

<style scoped>
.info-content {
  padding: 1rem 0;
}
</style>
