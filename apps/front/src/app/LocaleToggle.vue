<script setup lang="ts">
import { useI18n } from 'vue-i18n';
import {
  LOCALES,
  localePreference,
  resolvedLocale,
} from '../composables/usePreferences.ts';
import PreferenceGroup from './PreferenceGroup.vue';

const { t } = useI18n();
</script>

<template>
  <PreferenceGroup
    v-model="localePreference"
    :label="t('prefs.locale')"
    :options="
      LOCALES.map((value) => ({ value, label: t(`prefs.locales.${value}`) }))
    "
    :note="
      t('prefs.systemLocale', { value: t(`prefs.locales.${resolvedLocale}`) })
    "
  />
</template>
