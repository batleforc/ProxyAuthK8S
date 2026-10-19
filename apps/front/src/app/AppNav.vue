<script setup lang="ts">
import { computed } from 'vue';
import { RouterLink } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { useAuthStore } from '../store/auth.ts';

const { t } = useI18n();
const authStore = useAuthStore();

const items = computed(() => [
  { to: '/', label: t('nav.home') },
  ...(authStore.isAuthenticated ? [{ to: '/cli', label: t('nav.cli') }] : []),
  { to: '/about', label: t('nav.about') },
]);
</script>

<template>
  <!-- One segmented frame, not a row of links; the current page reverses to
       an ink block. It wraps under 640px instead of becoming a second nav. -->
  <nav
    :aria-label="t('nav.label')"
    class="flex flex-wrap border border-border"
  >
    <RouterLink
      v-for="item in items"
      :key="item.to"
      v-slot="{ href, navigate, isExactActive }"
      :to="item.to"
      custom
    >
      <a
        :href="href"
        :aria-current="isExactActive ? 'page' : undefined"
        class="border-r border-border px-3 py-2 text-xs uppercase tracking-[0.1em] text-muted-foreground no-underline last:border-r-0 hover:text-foreground aria-[current=page]:bg-foreground aria-[current=page]:font-bold aria-[current=page]:text-background"
        @click="navigate"
        >{{ item.label }}</a
      >
    </RouterLink>
  </nav>
</template>
