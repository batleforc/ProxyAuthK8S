<script setup lang="ts">
import MazBadge from 'maz-ui/components/MazBadge';
import { LazyMazServer } from '@maz-ui/icons';
import type { VisibleCluster } from '@proxy-auth-k8s/front-api';
import SectionCard from '../common/SectionCard.vue';
import InfoRow from '../cluster/InfoRow.vue';
import type { ClusterCallbackData } from '../../utils/kubeconfig.ts';

defineProps<{
  callbackData: ClusterCallbackData;
  currentCluster: VisibleCluster | undefined;
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
          {{ callbackData.cluster }}
        </MazBadge>
      </InfoRow>
      <InfoRow label="Namespace:">
        <MazBadge
          color="primary"
          size="sm"
        >
          {{ callbackData.ns }}
        </MazBadge>
      </InfoRow>
      <InfoRow
        label="URL de l'API:"
        :value="callbackData.retour.cluster_url"
      />
      <InfoRow
        label="Utilisateur:"
        :value="callbackData.retour.subject"
      />
      <InfoRow label="Status:">
        <MazBadge
          :color="currentCluster ? (currentCluster.enabled ? 'success' : 'destructive') : 'info'"
          size="sm"
        >
          {{ currentCluster ? (currentCluster.enabled ? 'Activé' : 'Désactivé') : 'Inconnu' }}
        </MazBadge>
      </InfoRow>
      <InfoRow label="Accessible:">
        <MazBadge
          :color="currentCluster && currentCluster.is_reachable !== null
            ? (currentCluster.is_reachable ? 'success' : 'warning')
            : 'info'
          "
          size="sm"
        >
          {{
            currentCluster && currentCluster.is_reachable !== null
              ? (currentCluster.is_reachable ? 'Oui' : 'Non')
              : 'Inconnu'
          }}
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
