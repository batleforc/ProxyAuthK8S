<script setup lang="ts">
import { computed } from 'vue';
import MazTabs from 'maz-ui/components/MazTabs';
import MazTabsBar from 'maz-ui/components/MazTabsBar';
import MazTabsContent from 'maz-ui/components/MazTabsContent';
import MazTabsContentItem from 'maz-ui/components/MazTabsContentItem';
import { useToast } from 'maz-ui/composables/useToast';
import { LazyMazCloudArrowDown } from '@maz-ui/icons';
import SectionCard from '../common/SectionCard.vue';
import KubeconfigBlock from '../cluster/KubeconfigBlock.vue';
import PluginCliSteps from './PluginCliSteps.vue';
import {
  type ClusterCallbackData,
  generateCallbackKubeconfig,
  generateCallbackPluginCommands,
  generateCallbackPluginKubeconfig,
} from '../../utils/kubeconfig.ts';
import { downloadTextFile } from '../../utils/download.ts';
import { useCopyToClipboard } from '../../composables/useCopyToClipboard.ts';

const props = defineProps<{
  callbackData: ClusterCallbackData;
}>();

const toast = useToast();
const copyToClipboard = useCopyToClipboard();

const kubeconfig = computed(() => generateCallbackKubeconfig(props.callbackData));
const pluginKubeconfig = computed(() => generateCallbackPluginKubeconfig(props.callbackData));
const pluginCommands = computed(() => generateCallbackPluginCommands(props.callbackData));

const downloadKubeconfig = () => {
  downloadTextFile(
    kubeconfig.value,
    `kubeconfig-${props.callbackData.ns}-${props.callbackData.cluster}.yaml`,
  );
  toast.success('Kubeconfig téléchargé avec succès !');
};

const downloadPluginKubeconfig = () => {
  downloadTextFile(
    pluginKubeconfig.value,
    `kubeconfig-plugin-${props.callbackData.ns}-${props.callbackData.cluster}.yaml`,
  );
  toast.success('Kubeconfig Plugin téléchargé avec succès !');
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
        Choisissez le type de configuration Kubernetes selon votre usage.
      </p>

      <MazTabs>
        <MazTabsBar
          :items="[
            { label: 'Kubeconfig Standard', disabled: false },
            { label: 'Kubeconfig Plugin (SOON in v0.2.0)', disabled: false },
            { label: 'Configuration via Plugin CLI (SOON in v0.2.0)', disabled: false }
          ]"
        />

        <MazTabsContent>
          <MazTabsContentItem :tab="1">
            <div class="tab-content">
              <p class="tab-description">
                Configuration avec token statique. Utilisez ce fichier kubeconfig pour vous connecter
                directement avec le token fourni.
              </p>

              <KubeconfigBlock
                :content="kubeconfig"
                download-label="Télécharger kubeconfig"
                copy-label="Copier kubeconfig"
                @download="downloadKubeconfig"
                @copy="copyToClipboard(kubeconfig, 'Kubeconfig')"
              />
            </div>
          </MazTabsContentItem>

          <MazTabsContentItem :tab="2">
            <div class="tab-content">
              <p class="tab-description">
                Configuration avec plugin kubectl-proxyauth. Les tokens sont automatiquement gérés et
                rafraîchis par le plugin.
              </p>

              <KubeconfigBlock
                :content="pluginKubeconfig"
                download-label="Télécharger kubeconfig plugin"
                copy-label="Copier kubeconfig plugin"
                @download="downloadPluginKubeconfig"
                @copy="copyToClipboard(pluginKubeconfig, 'Kubeconfig Plugin')"
              />
            </div>
          </MazTabsContentItem>

          <MazTabsContentItem :tab="3">
            <div class="tab-content">
              <p class="tab-description">
                Utilisez le plugin kubectl-proxyauth pour vous connecter directement via la ligne de
                commande.
                Cette méthode est recommandée pour une intégration transparente avec kubectl.
              </p>

              <PluginCliSteps :plugin-commands="pluginCommands" />
            </div>
          </MazTabsContentItem>
        </MazTabsContent>
      </MazTabs>
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

.tab-content {
  padding: 1rem 0;
}

.tab-description {
  color: #94a3b8;
  margin: 0 0 1rem 0;
  line-height: 1.6;
  font-size: 0.875rem;
}
</style>
