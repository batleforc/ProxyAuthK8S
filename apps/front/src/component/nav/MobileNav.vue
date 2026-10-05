<script setup lang="ts">
import { RouterLink } from 'vue-router';
import MazBtn from 'maz-ui/components/MazBtn';
import MazAvatar from 'maz-ui/components/MazAvatar';
import MazIcon from 'maz-ui/components/MazIcon';
import {
  MazArrowRightOnRectangle,
  MazCog6Tooth,
  MazArrowLeftOnRectangle
} from '@maz-ui/icons/static';
import type { NavItem, UserMenuItem } from './navTypes.ts';

/** Slide-down mobile menu (links + user actions) and its backdrop overlay. */
defineProps<{
  open: boolean;
  navItems: NavItem[];
  items: UserMenuItem[];
  isAuthenticated: boolean;
  displayName: string;
  initials: string;
}>();

const emit = defineEmits<{
  close: [];
  login: [];
  logout: [];
}>();
</script>

<template>
  <!-- Mobile Navigation -->
  <div
    class="mobile-nav"
    :class="{ open }"
  >
    <div class="mobile-nav-content">
      <!-- Mobile Navigation Links -->
      <div class="mobile-nav-links">
        <RouterLink
          v-for="item in navItems"
          :key="item.label"
          :to="item.to"
          class="mobile-nav-link"
          active-class="mobile-nav-link-active"
          @click="emit('close')"
        >
          <MazIcon
            :icon="item.icon"
            size="md"
            class="nav-icon"
          />
          <span>{{ item.label }}</span>
        </RouterLink>
      </div>

      <!-- Mobile User Section -->
      <div class="mobile-user-section">
        <div
          v-if="isAuthenticated"
          class="mobile-user-authenticated"
        >
          <div class="mobile-user-info">
            <MazAvatar
              :caption="initials"
              size="1rem"
              class="user-avatar"
            />
            <span class="user-name">{{ displayName }}</span>
          </div>

          <div class="mobile-user-actions">
            <template
              v-for="value in items"
              :key="value.label"
            >
              <MazBtn
                v-if="value.label !== 'Logout'"
                color="transparent"
                justify="start"
                block
                size="md"
                :left-icon="MazCog6Tooth"
                @click="() => { value.onClick(); emit('close'); }"
              >
                {{ value.label }}
              </MazBtn>
              <MazBtn
                v-else
                color="destructive"
                justify="start"
                block
                size="md"
                outlined
                :left-icon="MazArrowLeftOnRectangle"
                @click="emit('logout')"
              >
                Logout
              </MazBtn>
            </template>
          </div>
        </div>

        <div
          v-else
          class="mobile-user-unauthenticated"
        >
          <MazBtn
            color="success"
            block
            size="lg"
            :left-icon="MazArrowRightOnRectangle"
            @click="emit('login')"
          >
            Login
          </MazBtn>
        </div>
      </div>
    </div>
  </div>

  <!-- Mobile Menu Overlay -->
  <div
    v-if="open"
    class="mobile-overlay"
    @click="emit('close')"
  />
</template>

<style scoped>
/* Mobile Navigation */
.mobile-nav {
  display: none;
  position: fixed;
  top: 4rem;
  left: 0;
  right: 0;
  background: linear-gradient(135deg, #1e40af 0%, #7c3aed 100%);
  backdrop-filter: blur(12px);
  border-bottom: 1px solid rgba(255, 255, 255, 0.1);
  transform: translateY(-200%);
  transition: transform 0.3s cubic-bezier(0.4, 0, 0.2, 1);
  z-index: 45;
  max-height: calc(100vh - 4rem);
  overflow-y: auto;
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.15);
}

.mobile-nav.open {
  transform: translateY(0);
}

.mobile-nav-content {
  padding: 1.5rem;
  display: flex;
  flex-direction: column;
  gap: 2rem;
}

.mobile-nav-links {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.mobile-nav-link {
  display: flex;
  align-items: center;
  gap: 1rem;
  padding: 1rem;
  text-decoration: none;
  color: rgba(255, 255, 255, 0.9);
  font-weight: 500;
  border-radius: 0.75rem;
  transition: all 0.3s ease;
}

.mobile-nav-link:hover,
.mobile-nav-link-active {
  color: white;
  background: rgba(255, 255, 255, 0.15);
  transform: translateX(4px);
}

.nav-icon {
  display: flex;
  align-items: center;
  justify-content: center;
}

.user-name {
  color: white;
  font-weight: 500;
  font-size: 0.9rem;
}

.mobile-user-section {
  padding-top: 1rem;
  border-top: 1px solid rgba(255, 255, 255, 0.2);
}

.mobile-user-authenticated {
  display: flex;
  flex-direction: column;
  gap: 1rem;
}

.mobile-user-info {
  display: flex;
  align-items: center;
  gap: 1rem;
  padding: 0.5rem;
}

.mobile-user-actions {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.mobile-overlay {
  position: fixed;
  top: 4rem;
  left: 0;
  right: 0;
  bottom: 0;
  background: rgba(0, 0, 0, 0.5);
  z-index: 35;
  backdrop-filter: blur(2px);
}

@media (max-width: 768px) {
  .mobile-nav {
    display: block !important;
  }
}

@media (max-width: 480px) {
  .mobile-nav {
    top: 3.5rem;
    max-height: calc(100vh - 3.5rem);
  }

  .mobile-overlay {
    top: 3.5rem;
  }

  .mobile-nav-content {
    padding: 1rem;
  }
}

/* Dark mode compatible colors */
@media (prefers-color-scheme: dark) {
  .mobile-nav {
    background: linear-gradient(135deg, #1e293b 0%, #581c87 100%);
  }
}
</style>
