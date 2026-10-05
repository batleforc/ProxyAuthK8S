<script setup lang="ts">
import { RouterLink } from 'vue-router';
import MazIcon from 'maz-ui/components/MazIcon';
import type { NavItem } from './navTypes.ts';

defineProps<{
  navItems: NavItem[];
}>();
</script>

<template>
  <nav class="desktop-nav">
    <div class="nav-links">
      <RouterLink
        v-for="item in navItems"
        :key="item.label"
        :to="item.to"
        class="nav-link"
        active-class="nav-link-active"
      >
        <MazIcon
          :icon="item.icon"
          size="md"
          class="nav-icon"
        />
        <span>{{ item.label }}</span>
      </RouterLink>
    </div>
  </nav>
</template>

<style scoped>
/* Desktop Navigation */
.desktop-nav {
  display: flex;
  align-items: center;
  flex-grow: 1;
  justify-content: center;
}

.nav-links {
  display: flex;
  align-items: center;
  gap: 1rem;
}

.nav-link {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.75rem 1rem;
  text-decoration: none;
  color: rgba(255, 255, 255, 0.9);
  font-weight: 500;
  border-radius: 0.75rem;
  transition: all 0.3s ease;
  position: relative;
  font-size: 0.95rem;
}

.nav-link:hover {
  color: white;
  background: rgba(255, 255, 255, 0.15);
  transform: translateY(-2px);
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.15);
}

.nav-link-active {
  color: white;
  background: rgba(255, 255, 255, 0.2);
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.2);
}

.nav-icon {
  display: flex;
  align-items: center;
  justify-content: center;
}

@media (max-width: 768px) {
  .desktop-nav {
    display: none !important;
  }
}

/* Focus styles pour l'accessibilité */
.nav-link:focus {
  outline: 2px solid rgba(255, 255, 255, 0.5);
  outline-offset: 2px;
}
</style>
