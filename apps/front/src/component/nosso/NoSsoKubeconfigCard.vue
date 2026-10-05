<script setup lang="ts">
import { computed } from 'vue';
import { useToast } from 'maz-ui/composables/useToast';
import { LazyMazCloudArrowDown } from '@maz-ui/icons';
import type { VisibleCluster } from '@proxy-auth-k8s/front-api';
import SectionCard from '../common/SectionCard.vue';
import KubeconfigBlock from '../cluster/KubeconfigBlock.vue';
import { generateNoSsoKubeconfig } from '../../utils/kubeconfig.ts';
import { downloadTextFile } from '../../utils/download.ts';
import { useCopyToClipboard } from '../../composables/useCopyToClipboard.ts';

const props = defineProps<{
  cluster: VisibleCluster;
  clusterUrl: string;
  /** Name used for the kubeconfig user entry. */
  username: string;
}>();

const toast = useToast();
const copyToClipboard = useCopyToClipboard();

// Génération du kubeconfig
const kubeconfig = computed(() =>
  generateNoSsoKubeconfig(props.cluster, props.clusterUrl, props.username),
);

const downloadKubeconfig = () => {
  downloadTextFile(
    kubeconfig.value,
    `kubeconfig-${props.cluster.namespace}-${props.cluster.name}.yaml`,
  );
  toast.success('Kubeconfig téléchargé avec succès !');
};
</script>

<template>
  <SectionCard
    class="kubeconfig-card"
    title="Configuration Kubernetes"
    :icon="LazyMazCloudArrowDown"
  >
    <div class="kubeconfig-content">
      <p class="kubeconfig-description">
        Téléchargez la configuration Kubernetes pour accéder au cluster. Le kubeconfig utilise
        le proxy ProxyAuth comme endpoint et votre token d'authentification. Ce token est valide
        tant que votre session ProxyAuth est active.
      </p>

      <KubeconfigBlock
        framed
        :content="kubeconfig"
        download-label="Télécharger kubeconfig"
        copy-label="Copier kubeconfig"
        @download="downloadKubeconfig"
        @copy="copyToClipboard(kubeconfig, 'Kubeconfig')"
      />
    </div>
  </SectionCard>
</template>

<style scoped>
.kubeconfig-content {
  padding: 1rem 0;
}

.kubeconfig-description {
  color: #94a3b8;
  margin: 0 0 1.5rem 0;
  line-height: 1.6;
}
</style>
