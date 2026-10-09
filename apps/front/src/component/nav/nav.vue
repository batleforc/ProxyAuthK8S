<script setup lang="ts">
import { useRouter } from 'vue-router';
import { useAuthStore } from '../../store/auth.ts';
import { ref, computed } from 'vue';
import MazBtn from 'maz-ui/components/MazBtn';
import MazIcon from 'maz-ui/components/MazIcon';
// Import des icônes Maz-UI
import {
  MazHome,
  MazInformationCircle,
  MazBars3,
  MazXMark,
  MazArrowRightOnRectangle,
} from '@maz-ui/icons/static';
import NavBrand from './NavBrand.vue';
import DesktopNav from './DesktopNav.vue';
import UserMenu from './UserMenu.vue';
import MobileNav from './MobileNav.vue';
import type { NavItem, UserMenuItem } from './navTypes.ts';

const authStore = useAuthStore();
const isMobileMenuOpen = ref(false);
const router = useRouter();

const toggleMobileMenu = () => {
  isMobileMenuOpen.value = !isMobileMenuOpen.value;
};

const closeMobileMenu = () => {
  isMobileMenuOpen.value = false;
};

const handleLogin = () => {
  authStore.logIn();
  closeMobileMenu();
};

const handleLogout = () => {
  authStore.logOut();
  closeMobileMenu();
};

// Menu items pour le dropdown utilisateur
const userMenuItems = computed<UserMenuItem[]>(() => [
  {
    label: 'Cli',
    onClick: () => router.push('/cli'),
    class: 'dropdown-menu-item',
  },
  {
    label: 'Logout',
    onClick: handleLogout,
    class: 'dropdown-menu-item dropdown-menu-item-destructive',
  },
]);

// Navigation items
const navItems: NavItem[] = [
  {
    label: 'Home',
    to: '/',
    icon: MazHome,
  },
  {
    label: 'About',
    to: '/about',
    icon: MazInformationCircle,
  },
];

// Avatar caption pour l'utilisateur connecté
const userDisplayName = computed(() => {
  return authStore.getUserProfile?.profile?.name || 'User';
});

const userInitials = computed(() => {
  const name = userDisplayName.value;
  return name.charAt(0).toUpperCase();
});
</script>

<template>
  <header class="navbar">
    <div class="navbar-container">
      <!-- Logo/Brand -->
      <NavBrand @click="closeMobileMenu" />

      <!-- Desktop Navigation -->
      <DesktopNav :nav-items="navItems" />

      <!-- User Section -->
      <div class="user-section">
        <UserMenu
          v-if="authStore.isAuthenticated"
          :items="userMenuItems"
          :display-name="userDisplayName"
          :initials="userInitials"
        />

        <!-- Login button for unauthenticated users -->
        <div
          v-else
          class="user-unauthenticated"
        >
          <MazBtn
            color="success"
            size="md"
            :left-icon="MazArrowRightOnRectangle"
            @click="handleLogin"
          >
            <span class="desktop-only">Login</span>
          </MazBtn>
        </div>
      </div>

      <!-- Mobile Menu Button -->
      <MazBtn
        fab
        color="transparent"
        size="md"
        class="mobile-menu-btn"
        :class="{ active: isMobileMenuOpen }"
        @click="toggleMobileMenu"
      >
        <template #icon>
          <MazIcon
            :icon="isMobileMenuOpen ? MazXMark : MazBars3"
            size="md"
            style="color: white;"
          />
        </template>
      </MazBtn>
    </div>

    <MobileNav
      :open="isMobileMenuOpen"
      :nav-items="navItems"
      :items="userMenuItems"
      :is-authenticated="authStore.isAuthenticated"
      :display-name="userDisplayName"
      :initials="userInitials"
      @close="closeMobileMenu"
      @login="handleLogin"
      @logout="handleLogout"
    />
  </header>
</template>

<style scoped>
.navbar {
  position: sticky;
  top: 0;
  z-index: 50;
  background: linear-gradient(135deg, #1e40af 0%, #7c3aed 100%);
  backdrop-filter: blur(12px);
  border-bottom: 1px solid rgba(255, 255, 255, 0.1);
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.1);
}

.navbar-container {
  max-width: 1200px;
  margin: 0 auto;
  padding: 0 1.5rem;
  display: flex;
  align-items: center;
  justify-content: space-between;
  height: 4rem;
}

/* User Section */
.user-section {
  display: flex;
  align-items: center;
  gap: 1rem;
}

/* Mobile Menu Button */
.mobile-menu-btn {
  display: none !important;
  transition: all 0.3s ease;
  z-index: 51;
  position: relative;
}

.mobile-menu-btn:hover {
  transform: scale(1.05);
  background: rgba(255, 255, 255, 0.1) !important;
}

.mobile-menu-btn.active {
  background: rgba(255, 255, 255, 0.15) !important;
}

/* Responsive Design */
@media (max-width: 768px) {
  .mobile-menu-btn {
    display: flex !important;
    background: rgba(255, 255, 255, 0.1);
    border: 1px solid rgba(255, 255, 255, 0.2);
  }

  .navbar-container {
    padding: 0 1rem;
  }
}

@media (max-width: 480px) {
  .navbar-container {
    height: 3.5rem;
    padding: 0 0.75rem;
  }
}

/* Animation d'entrée */
@keyframes slideIn {
  from {
    opacity: 0;
    transform: translateY(-10px);
  }

  to {
    opacity: 1;
    transform: translateY(0);
  }
}

.navbar {
  animation: slideIn 0.4s ease-out;
}

/* Dark mode compatible colors */
@media (prefers-color-scheme: dark) {
  .navbar {
    background: linear-gradient(135deg, #1e293b 0%, #581c87 100%);
  }
}
</style>
