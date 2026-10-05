<script setup lang="ts">
import MazBtn from 'maz-ui/components/MazBtn';
import { RouterLink } from 'vue-router';
import { LazyMazInformationCircle as MazInformationCircle } from '@maz-ui/icons';
</script>

<template>
  <section class="cta-section">
    <div class="cta-container">
      <div class="cta-content">
        <h2 class="cta-title">
          Prêt à simplifier votre gestion Kubernetes ?
        </h2>
        <p class="cta-description">
          Rejoignez les équipes qui font confiance à ProxyAuthK8S pour sécuriser et centraliser l'accès à leurs
          clusters.
        </p>
        <div class="cta-actions">
          <RouterLink to="/about">
            <MazBtn
              color="transparent"
              size="xl"
              outlined
              class="cta-doc-button"
              :left-icon="MazInformationCircle"
            >
              Consulter la documentation
            </MazBtn>
          </RouterLink>
        </div>
      </div>
    </div>
  </section>
</template>

<style scoped>
/* CTA Section */
.cta-section {
  padding: 6rem 2rem;
  background: linear-gradient(135deg, #1e40af 0%, #7c3aed 100%);
  color: white;
  text-align: center;
}

.cta-container {
  max-width: 800px;
  margin: 0 auto;
}

.cta-title {
  font-size: 2.5rem;
  font-weight: 700;
  margin: 0 0 1.5rem 0;
  letter-spacing: -0.025em;
}

.cta-description {
  font-size: 1.25rem;
  line-height: 1.6;
  color: rgba(255, 255, 255, 0.9);
  margin: 0 0 2.5rem 0;
}

.cta-actions {
  display: flex;
  gap: 1.5rem;
  justify-content: center;
  flex-wrap: wrap;
}

.cta-doc-button {
  border-color: rgba(255, 255, 255, 0.4);
  color: white;
  box-shadow: 0 8px 32px rgba(255, 255, 255, 0.1);
}

.cta-doc-button:hover {
  background: rgba(255, 255, 255, 0.1);
  border-color: rgba(255, 255, 255, 0.6);
  transform: translateY(-3px);
  box-shadow: 0 12px 40px rgba(255, 255, 255, 0.15);
}

@media (max-width: 768px) {
  .cta-title {
    font-size: 2rem;
  }
}

@media (max-width: 480px) {
  .cta-section {
    padding: 4rem 1rem;
  }
}
</style>
