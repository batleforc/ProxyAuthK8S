<script setup lang="ts">
import MazIcon from 'maz-ui/components/MazIcon';
import { LazyMazServer, LazyMazShieldCheck } from '@maz-ui/icons';
import SectionCard from '../common/SectionCard.vue';

const authItems = [
  { icon: LazyMazShieldCheck, text: 'Pas de redirections SSO requises' },
  { icon: LazyMazServer, text: 'Requêtes via le proxy ProxyAuth' },
  { icon: LazyMazShieldCheck, text: 'Token ProxyAuth utilisé' },
];
</script>

<template>
  <SectionCard
    class="auth-card"
    title="Authentification"
    :icon="LazyMazShieldCheck"
  >
    <div class="auth-content">
      <p class="auth-description">
        L'accès à ce cluster se fait via votre token d'authentification ProxyAuth.
        Toutes les requêtes passent par le proxy qui gère l'authentification.
      </p>
      <div class="auth-details">
        <div
          v-for="item in authItems"
          :key="item.text"
          class="auth-item"
        >
          <MazIcon
            :icon="item.icon"
            size="sm"
            class="auth-icon"
          />
          <span>{{ item.text }}</span>
        </div>
      </div>
    </div>
  </SectionCard>
</template>

<style scoped>
.auth-content {
  padding: 1rem 0;
}

.auth-description {
  color: #94a3b8;
  margin: 0 0 1.5rem 0;
  line-height: 1.6;
}

.auth-details {
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
}

.auth-item {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  color: #cbd5e1;
  padding: 0.5rem;
  background: rgba(255, 255, 255, 0.05);
  border-radius: 6px;
}

.auth-icon {
  color: #06b6d4;
}
</style>
