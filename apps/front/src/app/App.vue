<script setup lang="ts">
import { RouterView } from 'vue-router';
import { useAuthStore } from '../store/auth.ts';
import Nav from '../component/nav/nav.vue';
import { onMounted, ref } from 'vue';
import MazSpinner from 'maz-ui/components/MazSpinner';

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
  <Nav />

  <!-- Loading state while auth is initializing -->
  <div v-if="isInitializing" class="app-loading">
    <div class="loading-content">
      <MazSpinner size="3rem" color="primary" />
      <p class="loading-text">Initialisation en cours...</p>
    </div>
  </div>

  <!-- Main app content -->
  <RouterView v-else />
</template>

<style scoped>
.app-loading {
  min-height: calc(100vh - 80px);
  display: flex;
  align-items: center;
  justify-content: center;
  background: linear-gradient(135deg, #0f172a 0%, #1e293b 100%);
}

.loading-content {
  text-align: center;
  color: #f1f5f9;
}

.loading-text {
  margin-top: 1rem;
  font-size: 1.125rem;
  color: #94a3b8;
}
</style>
