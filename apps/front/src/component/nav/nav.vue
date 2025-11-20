<script setup lang="ts">
import { RouterLink } from 'vue-router';
import { useAuthStore } from '../../store/auth.ts';
import { ref, computed } from 'vue';
import MazBtn from 'maz-ui/components/MazBtn';
import MazAvatar from 'maz-ui/components/MazAvatar';
import MazDropdown from 'maz-ui/components/MazDropdown';
import MazIcon from 'maz-ui/components/MazIcon';
// Import des icônes Maz-UI
import { 
  MazHome, 
  MazInformationCircle, 
  MazUser, 
  MazBars3, 
  MazXMark, 
  MazChevronDown,
  MazArrowRightOnRectangle,
  MazCog6Tooth,
  MazArrowLeftOnRectangle
} from '@maz-ui/icons';

const authStore = useAuthStore();
const isMobileMenuOpen = ref(false);

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
const userMenuItems = computed(() => [
  {
    label: 'Profile',
    onClick: () => console.log('Profile clicked'),
    class: 'dropdown-menu-item',
  },
  {
    label: 'Settings',
    onClick: () => console.log('Settings clicked'),
    class: 'dropdown-menu-item',
  },
  {
    label: 'Logout',
    onClick: handleLogout,
    class: 'dropdown-menu-item dropdown-menu-item-destructive',
  },
]);

// Navigation items
const navItems = [
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
      <div class="navbar-brand">
        <RouterLink to="/" class="brand-link" @click="closeMobileMenu">
          <div class="brand-icon">
            <svg viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
              <path d="M12 2L2 7L12 12L22 7L12 2Z" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
              <path d="M2 17L12 22L22 17" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
              <path d="M2 12L12 17L22 12" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
            </svg>
          </div>
          <span class="brand-text">ProxyAuthK8S</span>
        </RouterLink>
      </div>

      <!-- Desktop Navigation -->
      <nav class="desktop-nav">
        <div class="nav-links">
          <RouterLink 
            v-for="item in navItems"
            :key="item.label"
            :to="item.to"
            class="nav-link"
            active-class="nav-link-active"
          >
            <MazIcon :icon="item.icon" size="md" class="nav-icon" />
            <span>{{ item.label }}</span>
          </RouterLink>
        </div>
      </nav>

      <!-- User Section -->
      <div class="user-section">
        <div v-if="authStore.isAuthenticated" class="user-authenticated">
          <!-- User info with dropdown menu -->
          <MazDropdown 
            :items="userMenuItems"
            trigger="click"
            position="bottom-end"
            color="transparent"
            :chevron="false"
            class="user-dropdown"
            menu-panel-class="dropdown-panel"
            menu-panel-style="background: rgba(30, 41, 59, 0.95); backdrop-filter: blur(16px); border: 1px solid rgba(255, 255, 255, 0.2); border-radius: 12px; box-shadow: 0 20px 40px rgba(0, 0, 0, 0.3), 0 0 0 1px rgba(255, 255, 255, 0.1); min-width: 200px; padding: 8px;"
          >
            <template #trigger>
              <div class="user-trigger" tabindex="-1">
                <MazAvatar 
                  :caption="userInitials"
                  size="1rem"
                  clickable
                  hide-clickable-icon
                  class="user-avatar"
                />
                <span class="user-name desktop-only">{{ userDisplayName }}</span>
                <MazIcon :icon="MazChevronDown" size="sm" class="chevron-icon desktop-only" />
              </div>
            </template>
            
            <template #menuitem-label="{ item }">
              <div class="dropdown-item-content">
                <MazIcon 
                  v-if="item.label === 'Profile'"
                  :icon="MazUser" 
                  size="sm" 
                  class="dropdown-item-icon"
                />
                <MazIcon 
                  v-else-if="item.label === 'Settings'"
                  :icon="MazCog6Tooth" 
                  size="sm" 
                  class="dropdown-item-icon"
                />
                <MazIcon 
                  v-else-if="item.label === 'Logout'"
                  :icon="MazArrowLeftOnRectangle" 
                  size="sm" 
                  class="dropdown-item-icon dropdown-item-icon-destructive"
                />
                <span :class="{ 'dropdown-item-text-destructive': item.label === 'Logout' }">
                  {{ item.label }}
                </span>
              </div>
            </template>
          </MazDropdown>
        </div>
        
        <!-- Login button for unauthenticated users -->
        <div v-else class="user-unauthenticated">
          <MazBtn
            @click="handleLogin"
            color="success"
            size="md"
            :left-icon="MazArrowRightOnRectangle"
          >
            <span class="desktop-only">Login</span>
          </MazBtn>
        </div>
      </div>

      <!-- Mobile Menu Button -->
      <MazBtn
        fab
        @click="toggleMobileMenu"
        color="transparent"
        size="md"
        class="mobile-menu-btn"
        :class="{ active: isMobileMenuOpen }"
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

    <!-- Mobile Navigation -->
    <div class="mobile-nav" :class="{ open: isMobileMenuOpen }">
      <div class="mobile-nav-content">
        <!-- Mobile Navigation Links -->
        <div class="mobile-nav-links">
          <RouterLink
            v-for="item in navItems"
            :key="item.label"
            :to="item.to"
            class="mobile-nav-link"
            active-class="mobile-nav-link-active"
            @click="closeMobileMenu"
          >
            <MazIcon :icon="item.icon" size="md" class="nav-icon" />
            <span>{{ item.label }}</span>
          </RouterLink>
        </div>

        <!-- Mobile User Section -->
        <div class="mobile-user-section">
          <div v-if="authStore.isAuthenticated" class="mobile-user-authenticated">
            <div class="mobile-user-info">
              <MazAvatar 
                :caption="userInitials"
                size="1rem"
                class="user-avatar"
              />
              <span class="user-name">{{ userDisplayName }}</span>
            </div>
            
            <div class="mobile-user-actions">
              <MazBtn
                @click="() => { console.log('Profile'); closeMobileMenu(); }"
                color="transparent"
                justify="start"
                block
                size="md"
                :left-icon="MazUser"
              >
                Profile
              </MazBtn>
              
              <MazBtn
                @click="() => { console.log('Settings'); closeMobileMenu(); }"
                color="transparent"
                justify="start"
                block
                size="md"
                :left-icon="MazCog6Tooth"
              >
                Settings
              </MazBtn>
              
              <MazBtn
                @click="handleLogout"
                color="destructive"
                justify="start"
                block
                size="md"
                outlined
                :left-icon="MazArrowLeftOnRectangle"
              >
                Logout
              </MazBtn>
            </div>
          </div>
          
          <div v-else class="mobile-user-unauthenticated">
            <MazBtn
              @click="handleLogin"
              color="success"
              block
              size="lg"
              :left-icon="MazArrowRightOnRectangle"
            >
              Login
            </MazBtn>
          </div>
        </div>
      </div>
    </div>

    <!-- Mobile Menu Overlay -->
    <div 
      v-if="isMobileMenuOpen" 
      class="mobile-overlay" 
      @click="closeMobileMenu"
    ></div>
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

/* User Section */
.user-section {
  display: flex;
  align-items: center;
  gap: 1rem;
}

.user-authenticated {
  display: flex;
  align-items: center;
}

.user-dropdown {
  border-radius: 0.75rem;
}

.user-trigger {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  padding: 0.5rem;
  border-radius: 0.75rem;
  transition: all 0.3s ease;
  cursor: pointer;
}

.user-trigger:hover {
  background: rgba(255, 255, 255, 0.1);
  transform: translateY(-1px);
}

.user-name {
  color: white;
  font-weight: 500;
  font-size: 0.9rem;
}

.chevron-icon {
  color: rgba(255, 255, 255, 0.7);
  transition: transform 0.3s ease;
}

.user-trigger:hover .chevron-icon {
  transform: translateY(1px);
}

.login-btn {
  box-shadow: 0 4px 12px rgba(16, 185, 129, 0.3);
}

.login-btn:hover {
  transform: translateY(-2px);
  box-shadow: 0 6px 16px rgba(16, 185, 129, 0.4);
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

/* Responsive Design */
@media (max-width: 768px) {
  .desktop-nav {
    display: none !important;
  }

  .mobile-menu-btn {
    display: flex !important;
    background: rgba(255, 255, 255, 0.1);
    border: 1px solid rgba(255, 255, 255, 0.2);
  }

  .mobile-nav {
    display: block !important;
  }

  .desktop-only {
    display: none !important;
  }

  .navbar-container {
    padding: 0 1rem;
  }

  .brand-text {
    font-size: 1.125rem;
  }
}

@media (max-width: 480px) {
  .navbar-container {
    height: 3.5rem;
    padding: 0 0.75rem;
  }

  .mobile-nav {
    top: 3.5rem;
    max-height: calc(100vh - 3.5rem);
  }

  .mobile-overlay {
    top: 3.5rem;
  }

  .brand-icon {
    width: 1.75rem;
    height: 1.75rem;
  }

  .brand-text {
    font-size: 1rem;
  }

  .mobile-nav-content {
    padding: 1rem;
  }
}

/* Focus styles pour l'accessibilité */
.brand-link:focus,
.nav-link:focus,
.user-trigger:focus {
  outline: 2px solid rgba(255, 255, 255, 0.5);
  outline-offset: 2px;
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

/* Dropdown Menu Styles */
.dropdown-panel {
  background: rgba(30, 41, 59, 0.95) !important;
  backdrop-filter: blur(16px) !important;
  border: 1px solid rgba(255, 255, 255, 0.2) !important;
  border-radius: 12px !important;
  box-shadow: 0 20px 40px rgba(0, 0, 0, 0.3), 0 0 0 1px rgba(255, 255, 255, 0.1) !important;
  min-width: 200px !important;
  padding: 8px !important;
  margin-top: 8px !important;
}

.dropdown-menu-item {
  color: rgba(255, 255, 255, 0.9) !important;
  font-weight: 500 !important;
  border-radius: 8px !important;
  transition: all 0.2s ease !important;
  padding: 12px 16px !important;
  margin: 2px 0 !important;
}

.dropdown-menu-item:hover {
  background: linear-gradient(135deg, rgba(59, 130, 246, 0.8), rgba(147, 51, 234, 0.8)) !important;
  color: white !important;
  transform: translateX(2px) !important;
  box-shadow: 0 4px 12px rgba(59, 130, 246, 0.3) !important;
}

.dropdown-menu-item-destructive {
  color: #ef4444 !important;
}

.dropdown-menu-item-destructive:hover {
  background: rgba(239, 68, 68, 0.15) !important;
  color: #f87171 !important;
}

.dropdown-item-content {
  display: flex;
  align-items: center;
  gap: 12px;
  width: 100%;
}

.dropdown-menu-item:hover .dropdown-item-content {
  color: white;
}

.dropdown-item-icon {
  transition: color 0.2s ease;
}

.dropdown-item-icon-destructive {
  color: #ef4444;
}

.dropdown-item-text-destructive {
  color: #ef4444;
  font-weight: 500;
}

.dropdown-menu-item:hover .dropdown-item-icon {
  color: white;
}

.dropdown-menu-item-destructive:hover .dropdown-item-icon-destructive {
  color: #f87171;
}

/* Dark mode compatible colors */
@media (prefers-color-scheme: dark) {
  .navbar {
    background: linear-gradient(135deg, #1e293b 0%, #581c87 100%);
  }
  
  .mobile-nav {
    background: linear-gradient(135deg, #1e293b 0%, #581c87 100%);
  }
}
</style>