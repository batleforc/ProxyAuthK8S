<script setup lang="ts">
import { useAuthStore } from '../store/auth.ts';
import MazBtn from 'maz-ui/components/MazBtn';
import MazIcon from 'maz-ui/components/MazIcon';
import { RouterLink } from 'vue-router';
import {
  MazArrowRightOnRectangle,
  MazInformationCircle,
  MazServer,
  MazShieldCheck,
  MazUser,
  MazRectangleGroup,
  MazBolt,
  MazUsers,
  MazChartBar,
  MazCog6Tooth,
  MazArrowRight,
  MazPlay
} from '@maz-ui/icons';

const authStore = useAuthStore();

const handleLogin = () => {
  authStore.logIn();
};
</script>

<template>
  <div class="home-container">
    <!-- Hero Section -->
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
            <MazBtn color="success" size="lg" @click="handleLogin" class="cta-button"
              :left-icon="MazArrowRightOnRectangle">
              Commencer maintenant
            </MazBtn>
            <RouterLink to="/about">
              <MazBtn color="transparent" size="lg" outlined class="learn-more-button"
                :left-icon="MazInformationCircle">
                En savoir plus
              </MazBtn>
            </RouterLink>
          </div>
        </div>
        <div class="hero-visual">
          <div class="svg-container">
            <svg viewBox="0 0 600 400" xmlns="http://www.w3.org/2000/svg" class="architecture-diagram">
              <!-- Définitions pour les animations et gradients -->
              <defs>
                <!-- Gradient pour le proxy central -->
                <linearGradient id="proxyGradient" x1="0%" y1="0%" x2="100%" y2="100%">
                  <stop offset="0%" style="stop-color:#10b981;stop-opacity:1" />
                  <stop offset="100%" style="stop-color:#059669;stop-opacity:1" />
                </linearGradient>

                <!-- Gradient pour les clusters -->
                <linearGradient id="clusterGradient" x1="0%" y1="0%" x2="100%" y2="100%">
                  <stop offset="0%" style="stop-color:#3b82f6;stop-opacity:1" />
                  <stop offset="100%" style="stop-color:#1d4ed8;stop-opacity:1" />
                </linearGradient>

                <!-- Gradient pour l'utilisateur -->
                <linearGradient id="userGradient" x1="0%" y1="0%" x2="100%" y2="100%">
                  <stop offset="0%" style="stop-color:#8b5cf6;stop-opacity:1" />
                  <stop offset="100%" style="stop-color:#7c3aed;stop-opacity:1" />
                </linearGradient>

                <!-- Animation pour les lignes de connexion -->
                <animate id="pulseAnimation" attributeName="stroke-opacity" values="0.3;1;0.3" dur="2s"
                  repeatCount="indefinite" />
              </defs>

              <!-- Lignes de connexion animées -->
              <!-- User vers Proxy -->
              <line x1="80" y1="200" x2="250" y2="200" stroke="rgba(255,255,255,0.6)" stroke-width="3"
                stroke-dasharray="5,5" class="connection-line">
                <animate attributeName="stroke-dashoffset" values="0;10" dur="1.5s" repeatCount="indefinite" />
                <animate attributeName="stroke-opacity" values="0.4;1;0.4" dur="2s" repeatCount="indefinite" />
              </line>

              <!-- Proxy vers Clusters -->
              <line x1="380" y1="200" x2="480" y2="120" stroke="rgba(255,255,255,0.6)" stroke-width="3"
                stroke-dasharray="5,5" class="connection-line">
                <animate attributeName="stroke-dashoffset" values="0;10" dur="1.8s" repeatCount="indefinite" />
                <animate attributeName="stroke-opacity" values="0.4;1;0.4" dur="2.2s" repeatCount="indefinite" />
              </line>

              <line x1="380" y1="200" x2="480" y2="200" stroke="rgba(255,255,255,0.6)" stroke-width="3"
                stroke-dasharray="5,5" class="connection-line">
                <animate attributeName="stroke-dashoffset" values="0;10" dur="1.6s" repeatCount="indefinite" />
                <animate attributeName="stroke-opacity" values="0.4;1;0.4" dur="2.5s" repeatCount="indefinite" />
              </line>

              <line x1="380" y1="200" x2="480" y2="280" stroke="rgba(255,255,255,0.6)" stroke-width="3"
                stroke-dasharray="5,5" class="connection-line">
                <animate attributeName="stroke-dashoffset" values="0;10" dur="2s" repeatCount="indefinite" />
                <animate attributeName="stroke-opacity" values="0.4;1;0.4" dur="1.8s" repeatCount="indefinite" />
              </line>

              <!-- Utilisateur -->
              <g class="user-group" transform="translate(50, 170)">
                <circle cx="30" cy="30" r="30" fill="url(#userGradient)" stroke="rgba(255,255,255,0.3)" stroke-width="2"
                  class="user-circle">
                </circle>
                <!-- Icône utilisateur -->
                <path
                  d="M20 20 C20 16, 24 12, 30 12 C36 12, 40 16, 40 20 C40 24, 36 28, 30 28 C24 28, 20 24, 20 20 M18 38 C18 32, 23 28, 30 28 C37 28, 42 32, 42 38 L42 42 L18 42 Z"
                  fill="white" />
                <text x="30" y="55" text-anchor="middle" fill="white" font-size="12" font-weight="500">
                  Utilisateur
                </text>
              </g>

              <!-- ProxyAuthK8S Central -->
              <g class="proxy-group" transform="translate(250, 150)">
                <rect x="0" y="0" width="130" height="100" rx="15" ry="15" fill="url(#proxyGradient)"
                  stroke="rgba(255,255,255,0.4)" stroke-width="2" class="proxy-rect">
                </rect>
                <!-- Icône bouclier -->
                <path d="M45 20 L65 15 L85 20 L85 35 C85 50, 75 60, 65 65 C55 60, 45 50, 45 35 Z" fill="white"
                  opacity="0.9" />
                <!-- Coche dans le bouclier -->
                <path d="M58 40 L62 44 L72 34" stroke="#065f46" stroke-width="2.5" stroke-linecap="round"
                  stroke-linejoin="round" fill="none" />
                <text x="65" y="80" text-anchor="middle" fill="white" font-size="11" font-weight="600">
                  ProxyAuthK8S
                </text>
              </g>

              <!-- Clusters Kubernetes -->
              <!-- Cluster A -->
              <g class="cluster-group cluster-1" transform="translate(480, 90)">
                <rect x="0" y="0" width="100" height="60" rx="10" ry="10" fill="url(#clusterGradient)"
                  stroke="rgba(255,255,255,0.3)" stroke-width="2" class="cluster-rect">
                </rect>
                <!-- Icône serveur -->
                <rect x="15" y="15" width="70" height="8" rx="2" fill="white" opacity="0.9" />
                <rect x="15" y="27" width="70" height="8" rx="2" fill="white" opacity="0.7" />
                <rect x="15" y="39" width="70" height="8" rx="2" fill="white" opacity="0.5" />
                <text x="50" y="75" text-anchor="middle" fill="white" font-size="11" font-weight="500">
                  Cluster A
                </text>
              </g>

              <!-- Cluster B -->
              <g class="cluster-group cluster-2" transform="translate(480, 170)">
                <rect x="0" y="0" width="100" height="60" rx="10" ry="10" fill="url(#clusterGradient)"
                  stroke="rgba(255,255,255,0.3)" stroke-width="2" class="cluster-rect">
                </rect>
                <!-- Icône serveur -->
                <rect x="15" y="15" width="70" height="8" rx="2" fill="white" opacity="0.9" />
                <rect x="15" y="27" width="70" height="8" rx="2" fill="white" opacity="0.7" />
                <rect x="15" y="39" width="70" height="8" rx="2" fill="white" opacity="0.5" />
                <text x="50" y="75" text-anchor="middle" fill="white" font-size="11" font-weight="500">
                  Cluster B
                </text>
              </g>

              <!-- Cluster C -->
              <g class="cluster-group cluster-3" transform="translate(480, 250)">
                <rect x="0" y="0" width="100" height="60" rx="10" ry="10" fill="url(#clusterGradient)"
                  stroke="rgba(255,255,255,0.3)" stroke-width="2" class="cluster-rect">
                </rect>
                <!-- Icône serveur -->
                <rect x="15" y="15" width="70" height="8" rx="2" fill="white" opacity="0.9" />
                <rect x="15" y="27" width="70" height="8" rx="2" fill="white" opacity="0.7" />
                <rect x="15" y="39" width="70" height="8" rx="2" fill="white" opacity="0.5" />
                <text x="50" y="75" text-anchor="middle" fill="white" font-size="11" font-weight="500">
                  Cluster C
                </text>
              </g>
            </svg>
          </div>
        </div>
      </div>
    </section>

    <!-- Features Section -->
    <section class="features-section">
      <div class="features-container">
        <h2 class="section-title">Pourquoi choisir ProxyAuthK8S ?</h2>
        <div class="features-grid">
          <div class="feature-card">
            <div class="feature-icon">
              <MazIcon :icon="MazShieldCheck" size="xl" />
            </div>
            <h3 class="feature-title">Sécurité Renforcée</h3>
            <p class="feature-description">
              Authentification centralisée avec possibilité de granularisation des permissions pour chaque cluster
              Kubernetes.
            </p>
          </div>

          <div class="feature-card">
            <div class="feature-icon">
              <MazIcon :icon="MazRectangleGroup" size="xl" />
            </div>
            <h3 class="feature-title">Interface Unifiée</h3>
            <p class="feature-description">
              Une seule interface pour accéder à tous vos clusters. Fini la jonglerie entre différents outils et
              authentifications. Intégration possible avec d'autre portails.
            </p>
          </div>

          <div class="feature-card">
            <div class="feature-icon">
              <MazIcon :icon="MazBolt" size="xl" />
            </div>
            <h3 class="feature-title">Accès Instantané</h3>
            <p class="feature-description">
              Redirection automatique vers les APIs avec authentification SSO. Pas d'impersonate pour sécuriser et
              partitionner les accès.
            </p>
          </div>

          <div class="feature-card">
            <div class="feature-icon">
              <MazIcon :icon="MazUsers" size="xl" />
            </div>
            <h3 class="feature-title">Gestion d'Équipe</h3>
            <p class="feature-description">
              Contrôlez finement qui peut accéder à quoi. Permissions par utilisateur et par cluster avec délégation
              RBAC pour une sécurité optimale.
            </p>
          </div>

          <div class="feature-card">
            <div class="feature-icon">
              <MazIcon :icon="MazChartBar" size="xl" />
            </div>
            <h3 class="feature-title">Monitoring</h3>
            <p class="feature-description">
              Suivez les accès en temps réel. Auditez facilement l'utilisation de vos ressources Kubernetes.
            </p>
          </div>

          <div class="feature-card">
            <div class="feature-icon">
              <MazIcon :icon="MazCog6Tooth" size="xl" />
            </div>
            <h3 class="feature-title">Configuration Flexible</h3>
            <p class="feature-description">
              Adaptez ProxyAuthK8S à votre infrastructure. Support de multiples providers d'authentification et
              configurations.
            </p>
          </div>
        </div>
      </div>
    </section>

    <!-- How it works Section -->
    <section class="how-it-works-section">
      <div class="how-it-works-container">
        <h2 class="section-title">Comment ça fonctionne ?</h2>
        <div class="steps-container">
          <div class="step">
            <div class="step-number">1</div>
            <div class="step-content">
              <h3 class="step-title">Authentifiez-vous</h3>
              <p class="step-description">
                Connectez-vous au ProxyAuthK8S qui vous proposera les clusters accessibles selon vos permissions.
              </p>
            </div>
          </div>

          <div class="step-arrow">
            <MazIcon :icon="MazArrowRight" size="lg" />
          </div>

          <div class="step">
            <div class="step-number">2</div>
            <div class="step-content">
              <h3 class="step-title">Explorez vos clusters</h3>
              <p class="step-description">
                Découvrez la liste des APIs Kubernetes accessibles selon vos permissions et si elle sont compatible SSO.
              </p>
            </div>
          </div>

          <div class="step-arrow">
            <MazIcon :icon="MazArrowRight" size="lg" />
          </div>

          <div class="step">
            <div class="step-number">3</div>
            <div class="step-content">
              <h3 class="step-title">Accédez en un clic</h3>
              <p class="step-description">
                Cliquez sur le cluster souhaité. L'api vous redirige vers l'authentification SSO propre au cluster en
                question.
              </p>
            </div>
          </div>

          <div class="step-arrow">
            <MazIcon :icon="MazArrowRight" size="lg" />
          </div>

          <div class="step">
            <div class="step-number">4</div>
            <div class="step-content">
              <h3 class="step-title">Communiquez avec l'api kube</h3>
              <p class="step-description">
                Echanger avec le cluster kube a travers Kubectl, K9s ou tout autre client kube !
              </p>
            </div>
          </div>
        </div>
      </div>
    </section>

    <!-- CTA Section -->
    <section class="cta-section">
      <div class="cta-container">
        <div class="cta-content">
          <h2 class="cta-title">Prêt à simplifier votre gestion Kubernetes ?</h2>
          <p class="cta-description">
            Rejoignez les équipes qui font confiance à ProxyAuthK8S pour sécuriser et centraliser l'accès à leurs
            clusters.
          </p>
          <div class="cta-actions">
            <RouterLink to="/about">
              <MazBtn color="transparent" size="xl" outlined class="cta-doc-button" :left-icon="MazInformationCircle">
                Consulter la documentation
              </MazBtn>
            </RouterLink>
          </div>
        </div>
      </div>
    </section>
  </div>
</template>

<style scoped>
.home-container {
  min-height: 100vh;
  font-family: var(--font-family-sans, 'Inter', sans-serif);
}

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

/* SVG Visual Container */
.svg-container {
  position: relative;
  height: 500px;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 1rem;
}

.architecture-diagram {
  width: 100%;
  height: 100%;
  max-width: 800px;
  filter: drop-shadow(0 4px 20px rgba(0, 0, 0, 0.15));
}

/* SVG Animations et effets */
.user-circle {
  filter: drop-shadow(0 4px 12px rgba(139, 92, 246, 0.4));
}

.proxy-rect {
  filter: drop-shadow(0 6px 20px rgba(16, 185, 129, 0.4));
}

.cluster-rect {
  filter: drop-shadow(0 4px 16px rgba(59, 130, 246, 0.3));
}

.connection-line {
  filter: drop-shadow(0 0 4px rgba(255, 255, 255, 0.3));
}

/* Animation pour le texte */
.architecture-diagram text {
  font-family: var(--font-family-sans, 'Inter', sans-serif);
}

/* Animation des particules */
.data-flow circle {
  filter: drop-shadow(0 0 6px currentColor);
}

/* Features Section */
.features-section {
  padding: 6rem 2rem;
  background: linear-gradient(135deg, #f8fafc 0%, #e2e8f0 100%);
}

.features-container {
  max-width: 1200px;
  margin: 0 auto;
}

.section-title {
  font-size: 2.5rem;
  font-weight: 700;
  text-align: center;
  margin: 0 0 4rem 0;
  color: #1e293b;
  letter-spacing: -0.025em;
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

.cta-main-button {
  box-shadow: 0 12px 40px rgba(16, 185, 129, 0.4);
}

.cta-main-button:hover {
  transform: translateY(-3px);
  box-shadow: 0 20px 60px rgba(16, 185, 129, 0.5);
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

/* Responsive Design */
@media (max-width: 1024px) {
  .hero-content {
    grid-template-columns: 1fr;
    gap: 3rem;
    text-align: center;
  }

  .svg-container {
    height: 400px;
    padding: 1rem;
  }

  .steps-container {
    grid-template-columns: 1fr;
    gap: 2rem;
  }

  .step-arrow {
    transform: rotate(90deg);
  }
}

@media (max-width: 768px) {
  .hero-title {
    font-size: 2.5rem;
  }

  .hero-description {
    font-size: 1.125rem;
  }

  .section-title {
    font-size: 2rem;
  }

  .cta-title {
    font-size: 2rem;
  }

  .features-grid {
    grid-template-columns: 1fr;
  }

  .hero-actions {
    justify-content: center;
  }
}

@media (max-width: 480px) {
  .hero-section {
    padding: 4rem 1rem 3rem;
  }

  .features-section,
  .how-it-works-section,
  .cta-section {
    padding: 4rem 1rem;
  }

  .hero-title {
    font-size: 2rem;
  }

  .svg-container {
    height: 300px;
    padding: 0.5rem;
  }

  .architecture-diagram {
    max-width: 100%;
  }

  /* Ajuster la taille du texte sur mobile */
  .architecture-diagram text {
    font-size: 10px;
  }
}

/* Accessibility */
@media (prefers-reduced-motion: reduce) {
  .architecture-diagram * {
    animation: none !important;
    transition: none !important;
  }

  .architecture-diagram animateTransform,
  .architecture-diagram animate,
  .architecture-diagram animateMotion {
    display: none;
  }
}
</style>