<script setup lang="ts">
import { computed, onMounted, ref } from 'vue';
import { RouterLink, useRoute, useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import type { VisibleCluster } from '@proxy-auth-k8s/front-api';
import { useClustersStore } from '../store/clusters.ts';
import { useAuthStore } from '../store/auth.ts';
import { useToast } from '../composables/useToast.ts';
import { PageHeader } from '@/components/ui/page-header';
import { Skeleton } from '@/components/ui/skeleton';
import { EmptyState } from '@/components/ui/empty-state';
import { Button } from '@/components/ui/button';
import ClusterSummary from '../features/cluster/ClusterSummary.vue';
import KubeconfigPanel from '../features/cluster/KubeconfigPanel.vue';
import { clusterProxyUrl, clusterStatus } from '../features/cluster/status.ts';
import { generateNoSsoKubeconfig } from '../utils/kubeconfig.ts';

const { t } = useI18n();
const clustersStore = useClustersStore();
const authStore = useAuthStore();
const toast = useToast();
const route = useRoute();
const router = useRouter();

const isLoading = ref(true);
const clusterData = ref<VisibleCluster | null>(null);

onMounted(async () => {
  const ns = route.params.ns as string;
  const cluster = route.params.cluster as string;

  if (!ns || !cluster) {
    toast.error(t('noSso.missingParams'), { timeout: 5000 });
    setTimeout(() => router.push({ name: 'home' }), 2000);
    return;
  }
  if (!clustersStore.isInited) {
    toast.info(t('noSso.loadingClusters'), { timeout: 3000 });
    await clustersStore.fetchClusters(toast);
  }
  const foundCluster = clustersStore.getClusters.find(
    (c) => c.namespace === ns && c.name === cluster,
  );
  if (!foundCluster) {
    toast.error(t('noSso.notFoundToast'), { timeout: 5000 });
    setTimeout(() => router.push({ name: 'home' }), 2000);
    return;
  }
  if (foundCluster.sso_enabled) {
    toast.warning(t('noSso.usesSso'), { timeout: 3000 });
    setTimeout(() => router.push({ name: 'home' }), 2000);
    return;
  }
  clusterData.value = foundCluster;
  isLoading.value = false;
});

const clusterUrl = computed(() =>
  clusterData.value ? clusterProxyUrl(clusterData.value) : '',
);
const username = computed(
  () => authStore.user?.profile?.preferred_username || 'user',
);
</script>

<template>
  <section class="border-b border-rule-soft px-4 pb-5 pt-10 sm:px-6">
    <PageHeader
      variant="display"
      :title="t('noSso.title')"
      :description="
        t('noSso.description', {
          cluster: clusterData?.name || '…',
          ns: clusterData?.namespace || '…',
        })
      "
    />
  </section>

  <section
    v-if="isLoading"
    class="px-4 py-6 sm:px-6"
    aria-busy="true"
  >
    <p
      class="mb-4 mt-0 text-xs uppercase tracking-[0.1em] text-muted-foreground"
    >
      {{ t('noSso.loading') }}
    </p>
    <Skeleton :lines="3" />
  </section>

  <template v-else-if="clusterData">
    <section
      class="border-b border-rule-soft px-4 py-6 sm:px-6"
      aria-labelledby="ns-info"
    >
      <h2
        id="ns-info"
        class="mb-4 mt-0 text-xs font-normal uppercase tracking-[0.16em] text-muted-foreground"
      >
        {{ t('noSso.info') }}
      </h2>
      <p class="mb-4 mt-0 max-w-[64ch] text-sm text-muted-foreground">
        {{ t('noSso.explanation') }}
      </p>
      <ClusterSummary
        :name="clusterData.name"
        :namespace="clusterData.namespace"
        :url="clusterUrl"
        :url-label="t('cluster.proxyUrl')"
        :user="authStore.user?.profile?.preferred_username || 'N/A'"
        :status="clusterStatus(clusterData)"
      />
    </section>
    <section
      class="px-4 py-6 sm:px-6"
      aria-labelledby="ns-kubeconfig"
    >
      <h2
        id="ns-kubeconfig"
        class="mb-4 mt-0 text-xs font-normal uppercase tracking-[0.16em] text-muted-foreground"
      >
        {{ t('kubeconfig.title') }}
      </h2>
      <p class="mb-4 mt-0 max-w-[64ch] text-sm text-muted-foreground">
        {{ t('noSso.kubeconfigText') }}
      </p>
      <KubeconfigPanel
        :content="generateNoSsoKubeconfig(clusterData, clusterUrl, username)"
        :filename="`kubeconfig-${clusterData.namespace}-${clusterData.name}.yaml`"
        name="Kubeconfig"
      />
    </section>
  </template>

  <section
    v-else
    class="px-4 py-6 sm:px-6"
  >
    <EmptyState
      class="border-destructive [&_p:first-child]:text-destructive"
      :title="t('noSso.notFound')"
      :description="t('noSso.notFoundText')"
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
