<script setup lang="ts">
import { computed, onMounted, ref } from 'vue';
import { RouterLink } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { useClustersStore } from '../store/clusters.ts';
import { useAuthStore } from '../store/auth.ts';
import { useToast } from '../composables/useToast.ts';
import { PageHeader } from '@/components/ui/page-header';
import { Skeleton } from '@/components/ui/skeleton';
import { EmptyState } from '@/components/ui/empty-state';
import { Button } from '@/components/ui/button';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs';
import ClusterSummary from '../features/cluster/ClusterSummary.vue';
import TokensPanel from '../features/cluster/TokensPanel.vue';
import KubeconfigPanel from '../features/cluster/KubeconfigPanel.vue';
import PluginCliSteps from '../features/cluster/PluginCliSteps.vue';
import { clusterStatus } from '../features/cluster/status.ts';
import {
  generateCallbackKubeconfig,
  generateCallbackPluginCommands,
  generateCallbackPluginKubeconfig,
} from '../utils/kubeconfig.ts';

const { t } = useI18n();
const clustersStore = useClustersStore();
const authStore = useAuthStore();
const toast = useToast();

const isAuthenticating = ref(true);
const authenticationComplete = ref(false);

onMounted(async () => {
  // Never log `authStore.user`: it holds the id/access/refresh tokens.
  if (
    authStore.isInited &&
    authStore.isAuthenticated &&
    clustersStore.inited === false
  ) {
    toast.info(t('clusterCallback.fetching'), { timeout: 3000 });
    await clustersStore.fetchClusters(toast);
  }
  if (authStore.isInited && authStore.isAuthenticated && clustersStore.inited) {
    try {
      toast.info(t('clusterCallback.finishing'), { timeout: 3000 });
      await clustersStore.callBackFromCluster(toast);
      authenticationComplete.value = true;
    } catch (error) {
      toast.error(t('clusterCallback.failedToast'), { timeout: 5000 });
      console.error('Error during callback:', error);
    } finally {
      isAuthenticating.value = false;
    }
  }
});

const callbackData = computed(() => clustersStore.callBack);
const hasToken = computed(() => !!callbackData.value.retour?.access_token);
const currentCluster = computed(() =>
  clustersStore.getClusters.find(
    (cluster) =>
      cluster.namespace === callbackData.value.ns &&
      cluster.name === callbackData.value.cluster,
  ),
);
const base = computed(
  () => `${callbackData.value.ns}-${callbackData.value.cluster}`,
);
</script>

<template>
  <section class="border-b border-rule-soft px-4 pb-5 pt-10 sm:px-6">
    <PageHeader
      variant="display"
      :title="t('clusterCallback.title')"
      :description="
        t('clusterCallback.description', {
          cluster: callbackData.cluster || '…',
          ns: callbackData.ns || '…',
        })
      "
    />
  </section>

  <section
    v-if="isAuthenticating"
    class="px-4 py-6 sm:px-6"
    aria-busy="true"
  >
    <p
      class="mb-4 mt-0 text-xs uppercase tracking-[0.1em] text-muted-foreground"
    >
      {{ t('clusterCallback.waiting') }}
    </p>
    <Skeleton :lines="4" />
  </section>

  <template v-else-if="authenticationComplete && hasToken">
    <section
      class="border-b border-rule-soft px-4 py-6 sm:px-6"
      aria-labelledby="cc-info"
    >
      <h2
        id="cc-info"
        class="mb-4 mt-0 text-xs font-normal uppercase tracking-[0.16em] text-muted-foreground"
      >
        {{ t('clusterCallback.info') }}
      </h2>
      <ClusterSummary
        :name="callbackData.cluster"
        :namespace="callbackData.ns"
        :url="callbackData.retour.cluster_url"
        :url-label="t('cluster.apiUrl')"
        :user="callbackData.retour.subject"
        :status="currentCluster ? clusterStatus(currentCluster) : 'unknown'"
      />
    </section>
    <section
      class="border-b border-rule-soft px-4 py-6 sm:px-6"
      aria-labelledby="cc-tokens"
    >
      <h2
        id="cc-tokens"
        class="mb-4 mt-0 text-xs font-normal uppercase tracking-[0.16em] text-muted-foreground"
      >
        {{ t('clusterCallback.tokens') }}
      </h2>
      <TokensPanel :tokens="callbackData.retour" />
    </section>
    <section
      class="px-4 py-6 sm:px-6"
      aria-labelledby="cc-kubeconfig"
    >
      <h2
        id="cc-kubeconfig"
        class="mb-4 mt-0 text-xs font-normal uppercase tracking-[0.16em] text-muted-foreground"
      >
        {{ t('kubeconfig.title') }}
      </h2>
      <Tabs default-value="standard">
        <TabsList :aria-label="t('kubeconfig.title')">
          <TabsTrigger value="standard">
            {{ t('kubeconfig.tabs.standard') }}
          </TabsTrigger>
          <TabsTrigger value="plugin">
            {{ t('kubeconfig.tabs.plugin') }}
          </TabsTrigger>
          <TabsTrigger value="cli">
            {{ t('kubeconfig.tabs.cli') }}
          </TabsTrigger>
        </TabsList>
        <TabsContent value="standard">
          <p class="mb-4 mt-0 max-w-[64ch] text-sm text-muted-foreground">
            {{ t('kubeconfig.standardText') }}
          </p>
          <KubeconfigPanel
            :content="generateCallbackKubeconfig(callbackData)"
            :filename="`kubeconfig-${base}.yaml`"
            name="Kubeconfig"
          />
        </TabsContent>
        <TabsContent value="plugin">
          <p class="mb-4 mt-0 max-w-[64ch] text-sm text-muted-foreground">
            {{ t('kubeconfig.pluginText') }}
          </p>
          <KubeconfigPanel
            :content="generateCallbackPluginKubeconfig(callbackData)"
            :filename="`kubeconfig-plugin-${base}.yaml`"
            name="Kubeconfig plugin"
          />
        </TabsContent>
        <TabsContent value="cli">
          <p class="mb-4 mt-0 max-w-[64ch] text-sm text-muted-foreground">
            {{ t('kubeconfig.cliText') }}
          </p>
          <PluginCliSteps
            :plugin-commands="generateCallbackPluginCommands(callbackData, t)"
          />
        </TabsContent>
      </Tabs>
    </section>
  </template>

  <section
    v-else
    class="px-4 py-6 sm:px-6"
  >
    <EmptyState
      class="border-destructive [&_p:first-child]:text-destructive"
      :title="t('clusterCallback.failed')"
      :description="t('clusterCallback.failedText')"
    >
      <template #action>
        <Button as-child>
          <RouterLink :to="{ name: 'home' }">
            {{ t('common.backHome') }}
          </RouterLink>
        </Button>
      </template>
    </EmptyState>
  </section>
</template>
