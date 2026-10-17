<script setup lang="ts">
import { useI18n } from 'vue-i18n';
import {
  THEMES,
  resolvedTheme,
  themePreference,
} from '../composables/usePreferences.ts';
import PreferenceGroup from './PreferenceGroup.vue';

const { t } = useI18n();
</script>

<template>
  <PreferenceGroup
    v-model="themePreference"
    :label="t('prefs.theme')"
    :options="
      THEMES.map((value) => ({ value, label: t(`prefs.themes.${value}`) }))
    "
    :note="
      t('prefs.systemTheme', { value: t(`prefs.themes.${resolvedTheme}`) })
    "
  />
</template>
