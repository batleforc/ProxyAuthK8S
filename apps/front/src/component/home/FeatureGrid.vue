<script setup lang="ts">
import MazIcon from 'maz-ui/components/MazIcon';
import {
  LazyMazShieldCheck as MazShieldCheck,
  LazyMazRectangleGroup as MazRectangleGroup,
  LazyMazBolt as MazBolt,
  LazyMazUsers as MazUsers,
  LazyMazChartBar as MazChartBar,
  LazyMazCog6Tooth as MazCog6Tooth,
} from '@maz-ui/icons';
import SectionTitle from './SectionTitle.vue';

const features = [
  {
    icon: MazShieldCheck,
    title: 'Sécurité Renforcée',
    description: 'Authentification centralisée avec possibilité de granularisation des permissions pour chaque cluster Kubernetes.',
  },
  {
    icon: MazRectangleGroup,
    title: 'Interface Unifiée',
    description: 'Une seule interface pour accéder à tous vos clusters. Fini la jonglerie entre différents outils et authentifications. Intégration possible avec d\'autre portails.',
  },
  {
    icon: MazBolt,
    title: 'Accès Instantané',
    description: 'Redirection automatique vers les APIs avec authentification SSO. Pas d\'impersonate pour sécuriser et partitionner les accès.',
  },
  {
    icon: MazUsers,
    title: 'Gestion d\'Équipe',
    description: 'Contrôlez finement qui peut accéder à quoi. Permissions par utilisateur et par cluster avec délégation RBAC pour une sécurité optimale.',
  },
  {
    icon: MazChartBar,
    title: 'Monitoring',
    description: 'Suivez les accès en temps réel. Auditez facilement l\'utilisation de vos ressources Kubernetes.',
  },
  {
    icon: MazCog6Tooth,
    title: 'Configuration Flexible',
    description: 'Adaptez ProxyAuthK8S à votre infrastructure. Support de multiples providers d\'authentification et configurations.',
  },
];
</script>

<template>
  <section class="features-section">
    <div class="features-container">
      <SectionTitle>Pourquoi choisir ProxyAuthK8S ?</SectionTitle>
      <div class="features-grid">
        <div
          v-for="feature in features"
          :key="feature.title"
          class="feature-card"
        >
          <div class="feature-icon">
            <MazIcon
              :icon="feature.icon"
              size="xl"
            />
          </div>
          <h3 class="feature-title">
            {{ feature.title }}
          </h3>
          <p class="feature-description">
            {{ feature.description }}
          </p>
        </div>
      </div>
    </div>
  </section>
</template>

<style scoped>
/* Features Section */
.features-section {
  padding: 6rem 2rem;
  background: linear-gradient(135deg, #f8fafc 0%, #e2e8f0 100%);
}

.features-container {
  max-width: 1200px;
  margin: 0 auto;
}

.features-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(350px, 1fr));
  gap: 2rem;
}

.feature-card {
  background: white;
  padding: 2rem;
  border-radius: 16px;
  box-shadow: 0 4px 6px rgba(0, 0, 0, 0.05);
  border: 1px solid rgba(0, 0, 0, 0.05);
  transition: all 0.3s ease;
  text-align: center;
}

.feature-card:hover {
  transform: translateY(-4px);
  box-shadow: 0 20px 40px rgba(0, 0, 0, 0.1);
  border-color: rgba(124, 58, 237, 0.2);
}

.feature-icon {
  color: #7c3aed;
  margin-bottom: 1.5rem;
  display: flex;
  justify-content: center;
}

.feature-title {
  font-size: 1.25rem;
  font-weight: 600;
  margin: 0 0 1rem 0;
  color: #1e293b;
}

.feature-description {
  color: #64748b;
  line-height: 1.6;
  margin: 0;
}

@media (max-width: 768px) {
  .features-grid {
    grid-template-columns: 1fr;
  }
}

@media (max-width: 480px) {
  .features-section {
    padding: 4rem 1rem;
  }
}
</style>
