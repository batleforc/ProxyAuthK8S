<script setup lang="ts">
import MazAvatar from 'maz-ui/components/MazAvatar';
import MazDropdown from 'maz-ui/components/MazDropdown';
import MazIcon from 'maz-ui/components/MazIcon';
import {
  MazUser,
  MazChevronDown,
  MazCog6Tooth,
  MazArrowLeftOnRectangle
} from '@maz-ui/icons/static';
import type { UserMenuItem } from './navTypes.ts';

/** Desktop avatar + name trigger opening the user dropdown menu. */
defineProps<{
  items: UserMenuItem[];
  displayName: string;
  initials: string;
}>();
</script>

<template>
  <div class="user-authenticated">
    <!-- User info with dropdown menu -->
    <MazDropdown
      :items="items"
      trigger="click"
      position="bottom-end"
      color="transparent"
      :chevron="false"
      class="user-dropdown"
      menu-panel-class="dropdown-panel"
      menu-panel-style="background: rgba(30, 41, 59, 0.95); backdrop-filter: blur(16px); border: 1px solid rgba(255, 255, 255, 0.2); border-radius: 12px; box-shadow: 0 20px 40px rgba(0, 0, 0, 0.3), 0 0 0 1px rgba(255, 255, 255, 0.1); min-width: 200px; padding: 8px;"
    >
      <template #trigger>
        <div
          class="user-trigger"
          tabindex="-1"
        >
          <MazAvatar
            :caption="initials"
            size="1rem"
            clickable
            hide-clickable-icon
            class="user-avatar"
          />
          <span class="user-name desktop-only">{{ displayName }}</span>
          <MazIcon
            :icon="MazChevronDown"
            size="sm"
            class="chevron-icon desktop-only"
          />
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
</template>

<style scoped>
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

.user-trigger:focus {
  outline: 2px solid rgba(255, 255, 255, 0.5);
  outline-offset: 2px;
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
</style>
