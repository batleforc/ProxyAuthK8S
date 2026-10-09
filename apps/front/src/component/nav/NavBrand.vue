<script setup lang="ts">
import { RouterLink } from 'vue-router';
import BrandLogo from '../common/BrandLogo.vue';

const emit = defineEmits<{
  click: [];
}>();
</script>

<template>
  <div class="navbar-brand">
    <RouterLink
      to="/"
      class="brand-link"
      @click="emit('click')"
    >
      <div class="brand-icon">
        <BrandLogo />
      </div>
      <span class="brand-text">ProxyAuthK8S</span>
    </RouterLink>
  </div>
</template>

<style scoped>
/* Brand */
.navbar-brand {
  display: flex;
  align-items: center;
  flex-shrink: 0;
}

.brand-link {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  text-decoration: none;
  color: white;
  font-weight: 700;
  font-size: 1.25rem;
  transition: all 0.3s ease;
  padding: 0.5rem;
  border-radius: 0.5rem;
}

.brand-link:hover {
  transform: translateY(-2px);
  background: rgba(255, 255, 255, 0.1);
}

.brand-icon {
  width: 2rem;
  height: 2rem;
  color: white;
  display: flex;
  align-items: center;
  justify-content: center;
}

.brand-text {
  font-family: var(--font-family-sans);
  font-weight: 700;
  letter-spacing: -0.025em;
  background: linear-gradient(45deg, #fff, #e0e7ff);
  -webkit-background-clip: text;
  -webkit-text-fill-color: transparent;
  background-clip: text;
}

@media (max-width: 768px) {
  .brand-text {
    font-size: 1.125rem;
  }
}

@media (max-width: 480px) {
  .brand-icon {
    width: 1.75rem;
    height: 1.75rem;
  }

  .brand-text {
    font-size: 1rem;
  }
}

/* Focus styles pour l'accessibilité */
.brand-link:focus {
  outline: 2px solid rgba(255, 255, 255, 0.5);
  outline-offset: 2px;
}
</style>
