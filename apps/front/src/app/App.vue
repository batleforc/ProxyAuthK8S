<script setup lang="ts">
import { RouterLink, RouterView } from 'vue-router';
import { useAuthStore } from '../store/auth.ts';
import Nav from '../component/nav/nav.vue';
import { onMounted } from 'vue';
const authStore = useAuthStore();

onMounted(() => {
  if (!authStore.inited){
    authStore.init();
  }
});
</script>

<template>
  <Nav />
  <RouterView />
</template>
