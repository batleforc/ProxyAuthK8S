<script setup lang="ts">
import Wordmark from './Wordmark.vue';
import AppNav from './AppNav.vue';
import SettingsMenu from './SettingsMenu.vue';
import UserMenu from './UserMenu.vue';
</script>

<template>
  <!-- The masthead: wordmark, segmented nav, then the preferences and the
       user, on the sunk ground above a soft rule. It wraps instead of
       scrolling sideways. -->
  <header
    class="flex flex-wrap items-center gap-3 border-b border-rule-soft bg-ground-sunk px-4 py-3 sm:gap-6 sm:px-6"
  >
    <Wordmark />
    <AppNav />
    <div class="ml-auto flex flex-wrap items-center gap-3">
      <SettingsMenu />
      <UserMenu />
    </div>
  </header>
</template>
