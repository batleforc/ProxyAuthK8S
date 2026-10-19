<script lang="ts" setup>
import { computed, onMounted, ref } from 'vue';
import { useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import type { VisibleCluster } from '@proxy-auth-k8s/front-api';
import { useClustersStore } from '../store/clusters.ts';
import { useAuthStore } from '../store/auth.ts';
import { useToast } from '../composables/useToast.ts';
import { useCopyToClipboard } from '../composables/useCopyToClipboard.ts';
import { PageHeader } from '@/components/ui/page-header';
import { Button } from '@/components/ui/button';
import { Skeleton } from '@/components/ui/skeleton';
import { EmptyState } from '@/components/ui/empty-state';
import {
  Table,
  TableBody,
  TableCaption,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table';
import ClusterRow from '../features/cluster/ClusterRow.vue';
import ClusterSummary from '../features/cluster/ClusterSummary.vue';
import { clusterProxyUrl, clusterStatus } from '../features/cluster/status.ts';

const { t } = useI18n();
const clustersStore = useClustersStore();
const authStore = useAuthStore();
const toast = useToast();
const router = useRouter();
const copyToClipboard = useCopyToClipboard();

onMounted(() => {
  if (authStore.inited && authStore.user && clustersStore.inited === false) {
    clustersStore.fetchClusters(toast);
  }
});

const clusters = computed(() => clustersStore.getClusters);
const isLoading = computed(() => !clustersStore.isInited);
const key = (cluster: VisibleCluster) => `${cluster.namespace}/${cluster.name}`;

// The selected cluster carries the view's one filled action; the first
// reachable one is selected until the user picks another.
const picked = ref<string | null>(null);
const selected = computed(
  () =>
    clusters.value.find((cluster) => key(cluster) === picked.value) ??
    clusters.value.find((cluster) => clusterStatus(cluster) === 'available') ??
    clusters.value[0],
);
const canConnect = computed(
  () =>
    !!selected.value &&
    selected.value.enabled &&
    selected.value.is_reachable !== false,
);

function connect(cluster: VisibleCluster) {
  if (cluster.sso_enabled) {
    toast.info(t('home.connecting', { name: cluster.name }));
    clustersStore.redirectToLogin(cluster.namespace, cluster.name);
  } else {
    toast.info(t('home.redirectNoSso', { name: cluster.name }));
    router.push({
      name: 'cluster-nosso',
      params: { ns: cluster.namespace, cluster: cluster.name },
    });
  }
}
</script>

<template>
  <section class="border-b border-rule-soft px-4 pb-5 pt-10 sm:px-6">
    <PageHeader
      variant="poster"
      :description="t('home.intro')"
    >
      <template #title>
        {{
          t('home.greeting', {
            name: authStore.user?.profile?.name || t('user.fallback'),
          })
        }}
      </template>
    </PageHeader>
  </section>

  <section
    class="px-4 py-6 sm:px-6"
    aria-labelledby="clusters-title"
  >
    <h2
      id="clusters-title"
      class="mb-4 mt-0 text-xs font-normal uppercase tracking-[0.16em] text-muted-foreground"
    >
      {{ t('home.clustersTitle') }}
    </h2>

    <Skeleton
      v-if="isLoading"
      :lines="4"
    />
    <EmptyState
      v-else-if="clusters.length === 0"
      :title="t('home.empty.title')"
      :description="t('home.empty.description')"
    />
    <template v-else>
      <Table :label="t('home.clustersTitle')">
        <TableCaption>
          {{ t('home.count', { count: clusters.length }, clusters.length) }}
        </TableCaption>
        <TableHeader>
          <TableRow>
            <TableHead class="w-[9rem] pl-3">
              {{ t('cluster.statusLabel') }}
            </TableHead>
            <TableHead>{{ t('cluster.name') }}</TableHead>
            <TableHead>{{ t('cluster.namespace') }}</TableHead>
            <TableHead class="hidden md:table-cell"> SSO </TableHead>
            <TableHead class="text-right">
              <span class="sr-only">{{ t('cluster.selection') }}</span>
            </TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          <ClusterRow
            v-for="cluster in clusters"
            :key="key(cluster)"
            :cluster="cluster"
            :selected="selected !== undefined && key(selected) === key(cluster)"
            @select="picked = key(cluster)"
          />
        </TableBody>
      </Table>

      <section
        v-if="selected"
        class="mt-6 border-t border-border pt-5"
        aria-live="polite"
        :aria-label="t('home.detail', { name: selected.name })"
      >
        <h3
          class="m-0 mb-3 font-display text-sub font-bold uppercase tracking-[0.04em] text-foreground"
        >
          {{ selected.name }}
        </h3>
        <ClusterSummary
          :name="selected.name"
          :namespace="selected.namespace"
          :url="clusterProxyUrl(selected)"
          :url-label="t('cluster.proxyUrl')"
          :user="
            authStore.user?.profile?.preferred_username || t('user.fallback')
          "
          :status="clusterStatus(selected)"
        />
        <div class="mt-5 flex flex-wrap gap-3">
          <Button
            :disabled="!canConnect"
            @click="connect(selected)"
          >
            {{ t('home.connect') }}
          </Button>
          <Button
            variant="outline"
            @click="
              copyToClipboard(clusterProxyUrl(selected), t('cluster.proxyUrl'))
            "
          >
            {{ t('home.copyUrl') }}
          </Button>
        </div>
      </section>
    </template>
  </section>
</template>
