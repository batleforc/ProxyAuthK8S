<script setup lang="ts">
import { defineAsyncComponent } from 'vue';
import { useAuthStore } from '../store/auth.ts';

// One chunk per variant: a visitor never downloads the cluster table, a
// signed-in user never the landing page.
const HomeLoggedin = defineAsyncComponent(() => import('./HomeLoggedin.vue'));
const HomeLoggedout = defineAsyncComponent(() => import('./HomeLoggedout.vue'));

const authStore = useAuthStore();
</script>
<template>
  <HomeLoggedin v-if="authStore.getUserProfile !== null" />
  <HomeLoggedout v-else />
</template>
