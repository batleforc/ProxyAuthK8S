<script setup lang="ts">
import MazBtn from 'maz-ui/components/MazBtn';
import { RouterLink } from 'vue-router';
import {
  LazyMazArrowRightOnRectangle as MazArrowRightOnRectangle,
  LazyMazInformationCircle as MazInformationCircle,
} from '@maz-ui/icons';
import { useAuthStore } from '../../store/auth.ts';
import ArchitectureDiagram from './ArchitectureDiagram.vue';

const authStore = useAuthStore();

const handleLogin = () => {
  authStore.logIn();
};
</script>

<template>
  <section class="hero-section">
    <div class="hero-content">
      <div class="hero-text">
        <h1 class="hero-title">
          <span class="title-highlight">ProxyAuthK8S</span>
          <br>
          Votre passerelle sécurisée vers Kubernetes
        </h1>
        <p class="hero-description">
          Centralisez et sécurisez l'accès à vos APIs Kubernetes.
          Une interface unique pour faciliter l'accés a vos différents cluster.
        </p>
        <div class="hero-actions">
          <MazBtn
            color="success"
            size="lg"
            class="cta-button"
            :left-icon="MazArrowRightOnRectangle"
            @click="handleLogin"
          >
            Commencer maintenant
          </MazBtn>
          <RouterLink to="/about">
            <MazBtn
              color="transparent"
              size="lg"
              outlined
              class="learn-more-button"
              :left-icon="MazInformationCircle"
            >
              En savoir plus
            </MazBtn>
          </RouterLink>
        </div>
      </div>
      <div class="hero-visual">
        <ArchitectureDiagram />
      </div>
    </div>
  </section>
</template>

<style scoped>
/* Hero Section */
.hero-section {
  background: linear-gradient(135deg, #1e40af 0%, #7c3aed 100%);
  padding: 6rem 2rem 4rem;
  color: white;
  position: relative;
  overflow: hidden;
}

.hero-section::before {
  content: '';
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  bottom: 0;
  background:
    radial-gradient(circle at 20% 80%, rgba(120, 119, 198, 0.3) 0%, transparent 50%),
    radial-gradient(circle at 80% 20%, rgba(255, 255, 255, 0.1) 0%, transparent 50%);
  pointer-events: none;
}

.hero-content {
  max-width: 1200px;
  margin: 0 auto;
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 4rem;
  align-items: center;
  position: relative;
  z-index: 1;
}

.hero-title {
  font-size: 3.5rem;
  font-weight: 800;
  line-height: 1.1;
  margin: 0 0 1.5rem 0;
  letter-spacing: -0.025em;
}

.title-highlight {
  background: linear-gradient(45deg, #60a5fa, #a78bfa);
  -webkit-background-clip: text;
  -webkit-text-fill-color: transparent;
  background-clip: text;
}

.hero-description {
  font-size: 1.25rem;
  line-height: 1.6;
  color: rgba(255, 255, 255, 0.9);
  margin: 0 0 2.5rem 0;
}

.hero-actions {
  display: flex;
  gap: 1rem;
  flex-wrap: wrap;
}

.cta-button {
  box-shadow: 0 8px 32px rgba(16, 185, 129, 0.4);
}

.cta-button:hover {
  transform: translateY(-2px);
  box-shadow: 0 12px 40px rgba(16, 185, 129, 0.5);
}

.learn-more-button {
  border-color: rgba(255, 255, 255, 0.3);
  color: white;
}

.learn-more-button:hover {
  background: rgba(255, 255, 255, 0.1);
  border-color: rgba(255, 255, 255, 0.5);
  transform: translateY(-2px);
}

@media (max-width: 1024px) {
  .hero-content {
    grid-template-columns: 1fr;
    gap: 3rem;
    text-align: center;
  }
}

@media (max-width: 768px) {
  .hero-title {
    font-size: 2.5rem;
  }

  .hero-description {
    font-size: 1.125rem;
  }

  .hero-actions {
    justify-content: center;
  }
}

@media (max-width: 480px) {
  .hero-section {
    padding: 4rem 1rem 3rem;
  }

  .hero-title {
    font-size: 2rem;
  }
}
</style>
