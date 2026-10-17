<script setup lang="ts">
import { onMounted, ref } from 'vue';
import { RouterView } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { useAuthStore } from '../store/auth.ts';
import AppLayout from '../layouts/AppLayout.vue';
import { Skeleton } from '@/components/ui/skeleton';

const { t } = useI18n();
const authStore = useAuthStore();
const isInitializing = ref(true);

onMounted(async () => {
  if (!authStore.inited) {
    await authStore.init();
  }
  isInitializing.value = false;
});
</script>

<template>
  <AppLayout>
    <section
      v-if="isInitializing"
      class="px-4 py-10 sm:px-6"
      :aria-label="t('app.initializing')"
      aria-busy="true"
    >
      <p class="mb-4 text-xs uppercase tracking-[0.1em] text-muted-foreground">
        {{ t('app.initializing') }}
      </p>
      <Skeleton :lines="3" />
    </section>
    <RouterView v-else />
  </AppLayout>
</template>
