<script setup lang="ts">
import { useI18n } from 'vue-i18n';
import AppHeader from '../app/AppHeader.vue';
import Notices from '../app/Notices.vue';

const { t } = useI18n();
</script>

<template>
  <!-- The one shell of every view: no centred max-width container, regions
       separated by full-width rules (The No-Container Rule). -->
  <a
    href="#main"
    class="absolute -left-[9999px] top-0 z-[100] bg-primary px-4 py-3 text-primary-foreground focus:left-0"
    >{{ t('a11y.skip') }}</a
  >
  <AppHeader />
  <main
    id="main"
    tabindex="-1"
    class="min-w-0"
  >
    <slot />
  </main>
  <Notices />
</template>
