<script setup lang="ts">
// Animated "user -> ProxyAuthK8S -> clusters" diagram of the landing hero.

// Lignes de connexion animées (user -> proxy, then proxy -> each cluster)
const connections = [
  { x1: '80', y1: '200', x2: '250', y2: '200', dashDur: '1.5s', opacityDur: '2s' },
  { x1: '380', y1: '200', x2: '480', y2: '120', dashDur: '1.8s', opacityDur: '2.2s' },
  { x1: '380', y1: '200', x2: '480', y2: '200', dashDur: '1.6s', opacityDur: '2.5s' },
  { x1: '380', y1: '200', x2: '480', y2: '280', dashDur: '2s', opacityDur: '1.8s' },
];

// Clusters Kubernetes
const clusters = [
  { label: 'Cluster A', className: 'cluster-1', transform: 'translate(480, 90)' },
  { label: 'Cluster B', className: 'cluster-2', transform: 'translate(480, 170)' },
  { label: 'Cluster C', className: 'cluster-3', transform: 'translate(480, 250)' },
];

// Icône serveur: three bars fading out
const serverBars = [
  { y: '15', opacity: '0.9' },
  { y: '27', opacity: '0.7' },
  { y: '39', opacity: '0.5' },
];
</script>

<template>
  <div class="svg-container">
    <svg
      viewBox="0 0 600 400"
      xmlns="http://www.w3.org/2000/svg"
      class="architecture-diagram"
    >
      <!-- Définitions pour les animations et gradients -->
      <defs>
        <!-- Gradient pour le proxy central -->
        <linearGradient
          id="proxyGradient"
          x1="0%"
          y1="0%"
          x2="100%"
          y2="100%"
        >
          <stop
            offset="0%"
            style="stop-color:#10b981;stop-opacity:1"
          />
          <stop
            offset="100%"
            style="stop-color:#059669;stop-opacity:1"
          />
        </linearGradient>

        <!-- Gradient pour les clusters -->
        <linearGradient
          id="clusterGradient"
          x1="0%"
          y1="0%"
          x2="100%"
          y2="100%"
        >
          <stop
            offset="0%"
            style="stop-color:#3b82f6;stop-opacity:1"
          />
          <stop
            offset="100%"
            style="stop-color:#1d4ed8;stop-opacity:1"
          />
        </linearGradient>

        <!-- Gradient pour l'utilisateur -->
        <linearGradient
          id="userGradient"
          x1="0%"
          y1="0%"
          x2="100%"
          y2="100%"
        >
          <stop
            offset="0%"
            style="stop-color:#8b5cf6;stop-opacity:1"
          />
          <stop
            offset="100%"
            style="stop-color:#7c3aed;stop-opacity:1"
          />
        </linearGradient>

        <!-- Animation pour les lignes de connexion -->
        <animate
          id="pulseAnimation"
          attributeName="stroke-opacity"
          values="0.3;1;0.3"
          dur="2s"
          repeatCount="indefinite"
        />
      </defs>

      <line
        v-for="line in connections"
        :key="`${line.x2}-${line.y2}`"
        :x1="line.x1"
        :y1="line.y1"
        :x2="line.x2"
        :y2="line.y2"
        stroke="rgba(255,255,255,0.6)"
        stroke-width="3"
        stroke-dasharray="5,5"
        class="connection-line"
      >
        <animate
          attributeName="stroke-dashoffset"
          values="0;10"
          :dur="line.dashDur"
          repeatCount="indefinite"
        />
        <animate
          attributeName="stroke-opacity"
          values="0.4;1;0.4"
          :dur="line.opacityDur"
          repeatCount="indefinite"
        />
      </line>

      <!-- Utilisateur -->
      <g
        class="user-group"
        transform="translate(50, 170)"
      >
        <circle
          cx="30"
          cy="30"
          r="30"
          fill="url(#userGradient)"
          stroke="rgba(255,255,255,0.3)"
          stroke-width="2"
          class="user-circle"
        />
        <!-- Icône utilisateur -->
        <path
          d="M20 20 C20 16, 24 12, 30 12 C36 12, 40 16, 40 20 C40 24, 36 28, 30 28 C24 28, 20 24, 20 20 M18 38 C18 32, 23 28, 30 28 C37 28, 42 32, 42 38 L42 42 L18 42 Z"
          fill="white"
        />
        <text
          x="30"
          y="55"
          text-anchor="middle"
          fill="white"
          font-size="12"
          font-weight="500"
        >
          Utilisateur
        </text>
      </g>

      <!-- ProxyAuthK8S Central -->
      <g
        class="proxy-group"
        transform="translate(250, 150)"
      >
        <rect
          x="0"
          y="0"
          width="130"
          height="100"
          rx="15"
          ry="15"
          fill="url(#proxyGradient)"
          stroke="rgba(255,255,255,0.4)"
          stroke-width="2"
          class="proxy-rect"
        />
        <!-- Icône bouclier -->
        <path
          d="M45 20 L65 15 L85 20 L85 35 C85 50, 75 60, 65 65 C55 60, 45 50, 45 35 Z"
          fill="white"
          opacity="0.9"
        />
        <!-- Coche dans le bouclier -->
        <path
          d="M58 40 L62 44 L72 34"
          stroke="#065f46"
          stroke-width="2.5"
          stroke-linecap="round"
          stroke-linejoin="round"
          fill="none"
        />
        <text
          x="65"
          y="80"
          text-anchor="middle"
          fill="white"
          font-size="11"
          font-weight="600"
        >
          ProxyAuthK8S
        </text>
      </g>

      <g
        v-for="cluster in clusters"
        :key="cluster.label"
        class="cluster-group"
        :class="cluster.className"
        :transform="cluster.transform"
      >
        <rect
          x="0"
          y="0"
          width="100"
          height="60"
          rx="10"
          ry="10"
          fill="url(#clusterGradient)"
          stroke="rgba(255,255,255,0.3)"
          stroke-width="2"
          class="cluster-rect"
        />
        <rect
          v-for="bar in serverBars"
          :key="bar.y"
          x="15"
          :y="bar.y"
          width="70"
          height="8"
          rx="2"
          fill="white"
          :opacity="bar.opacity"
        />
        <text
          x="50"
          y="75"
          text-anchor="middle"
          fill="white"
          font-size="11"
          font-weight="500"
        >
          {{ cluster.label }}
        </text>
      </g>
    </svg>
  </div>
</template>

<style scoped>
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

@media (max-width: 1024px) {
  .svg-container {
    height: 400px;
    padding: 1rem;
  }
}

@media (max-width: 480px) {
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
