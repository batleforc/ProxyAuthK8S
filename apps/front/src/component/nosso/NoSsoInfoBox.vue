<script setup lang="ts">
import { LazyMazInformationCircle } from '@maz-ui/icons';
import SectionCard from '../common/SectionCard.vue';
</script>

<template>
  <SectionCard
    class="info-box"
    title="À propos de ce cluster"
    :icon="LazyMazInformationCircle"
  >
    <div class="info-box-content">
      <p class="info-message">
        Ce cluster n'utilise pas d'authentification SSO/OIDC. L'accès se fait via le proxy ProxyAuth
        en utilisant votre token d'authentification actuel. Le kubeconfig généré utilisera le proxy
        comme point d'entrée pour toutes les requêtes vers le cluster.
      </p>
    </div>
  </SectionCard>
</template>

<style scoped>
/* Info Box: cyan variant of the dark section card. */
.section-card.info-box {
  background: rgba(6, 182, 212, 0.1);
  border-left: 4px solid #06b6d4;
}

.info-box-content {
  padding: 1rem;
}

.info-message {
  color: #cbd5e1;
  line-height: 1.6;
  margin: 0;
}
</style>
