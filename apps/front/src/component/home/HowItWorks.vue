<script setup lang="ts">
import MazIcon from 'maz-ui/components/MazIcon';
import { LazyMazArrowRight as MazArrowRight } from '@maz-ui/icons';
import SectionTitle from './SectionTitle.vue';

const steps = [
  {
    title: 'Authentifiez-vous',
    description: 'Connectez-vous au ProxyAuthK8S qui vous proposera les clusters accessibles selon vos permissions.',
  },
  {
    title: 'Explorez vos clusters',
    description: 'Découvrez la liste des APIs Kubernetes accessibles selon vos permissions et si elle sont compatible SSO.',
  },
  {
    title: 'Accédez en un clic',
    description: 'Cliquez sur le cluster souhaité. L\'api vous redirige vers l\'authentification SSO propre au cluster en question.',
  },
  {
    title: 'Communiquez avec l\'api kube',
    description: 'Echanger avec le cluster kube a travers Kubectl, K9s ou tout autre client kube !',
  },
];
</script>

<template>
  <section class="how-it-works-section">
    <div class="how-it-works-container">
      <SectionTitle>Comment ça fonctionne ?</SectionTitle>
      <div class="steps-container">
        <template
          v-for="(step, index) in steps"
          :key="step.title"
        >
          <div
            v-if="index > 0"
            class="step-arrow"
          >
            <MazIcon
              :icon="MazArrowRight"
              size="lg"
            />
          </div>

          <div class="step">
            <div class="step-number">
              {{ index + 1 }}
            </div>
            <div class="step-content">
              <h3 class="step-title">
                {{ step.title }}
              </h3>
              <p class="step-description">
                {{ step.description }}
              </p>
            </div>
          </div>
        </template>
      </div>
    </div>
  </section>
</template>

<style scoped>
/* How it works Section */
.how-it-works-section {
  padding: 6rem 2rem;
  background: white;
}

.how-it-works-container {
  max-width: 1000px;
  margin: 0 auto;
}

.steps-container {
  display: grid;
  grid-template-columns: 1fr auto 1fr auto 1fr auto 1fr;
  gap: 2rem;
  align-items: center;
}

.step {
  text-align: center;
  padding: 2rem;
}

.step-number {
  width: 60px;
  height: 60px;
  border-radius: 50%;
  background: linear-gradient(135deg, #1e40af, #7c3aed);
  color: white;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 1.5rem;
  font-weight: 700;
  margin: 0 auto 1.5rem;
  box-shadow: 0 8px 32px rgba(124, 58, 237, 0.3);
}

.step-title {
  font-size: 1.25rem;
  font-weight: 600;
  margin: 0 0 1rem 0;
  color: #1e293b;
}

.step-description {
  color: #64748b;
  line-height: 1.6;
  margin: 0;
}

.step-arrow {
  color: #7c3aed;
  display: flex;
  align-items: center;
  justify-content: center;
}

@media (max-width: 1024px) {
  .steps-container {
    grid-template-columns: 1fr;
    gap: 2rem;
  }

  .step-arrow {
    transform: rotate(90deg);
  }
}

@media (max-width: 480px) {
  .how-it-works-section {
    padding: 4rem 1rem;
  }
}
</style>
