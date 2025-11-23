<script setup lang="ts">
import { useAuthStore } from '../store/auth.ts';
import HomeLoggedin from './HomeLoggedin.vue';
import HomeLoggedout from './HomeLoggedout.vue';

const authStore = useAuthStore();
</script>

<template>
  <main>
    <HomeLoggedin v-if="authStore.getUserProfile !== null" />
    <HomeLoggedout v-else />
  </main>
</template>
