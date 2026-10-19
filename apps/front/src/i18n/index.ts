import { createI18n } from 'vue-i18n';
import en from './en.json';
import fr from './fr.json';
import { resolvedLocale } from '../composables/usePreferences.ts';

// Composition API only; messages are compiled just in time (no eval), so the
// CSP needs no 'unsafe-eval'.
export const i18n = createI18n({
  legacy: false,
  locale: resolvedLocale.value,
  fallbackLocale: 'en',
  messages: { en, fr },
});
