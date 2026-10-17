import { ref, watch } from 'vue';

/**
 * The two browser preferences (RFC 0002 §4.1). The Stored-Preference Rule:
 * store `system|light|dark` and `system|en|fr`, never the resolved value, and
 * re-resolve on an environment change only while the preference is System. A
 * value outside the list, or an unavailable storage, gives the default.
 */
export const THEMES = ['system', 'light', 'dark'] as const;
export const LOCALES = ['system', 'en', 'fr'] as const;
export type ThemePreference = (typeof THEMES)[number];
export type LocalePreference = (typeof LOCALES)[number];

const THEME_KEY = 'pak.theme';
const LOCALE_KEY = 'pak.locale';

function read<T extends string>(key: string, allowed: readonly T[]): T {
  try {
    const value = localStorage.getItem(key);
    if (value && (allowed as readonly string[]).includes(value))
      return value as T;
  } catch {
    // Private mode: session-only preferences, never an error.
  }
  return allowed[0];
}

function write(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    // Same: the choice holds for this page only.
  }
}

export function resolveTheme(pref: ThemePreference): 'light' | 'dark' {
  if (pref !== 'system') return pref;
  return globalThis.matchMedia?.('(prefers-color-scheme: light)').matches
    ? 'light'
    : 'dark';
}

export function resolveLocale(pref: LocalePreference): 'en' | 'fr' {
  if (pref !== 'system') return pref;
  return navigator.language?.toLowerCase().startsWith('fr') ? 'fr' : 'en';
}

export const themePreference = ref<ThemePreference>(read(THEME_KEY, THEMES));
export const localePreference = ref<LocalePreference>(
  read(LOCALE_KEY, LOCALES),
);
export const resolvedTheme = ref(resolveTheme(themePreference.value));
export const resolvedLocale = ref(resolveLocale(localePreference.value));

function applyTheme() {
  resolvedTheme.value = resolveTheme(themePreference.value);
  document.documentElement.setAttribute('data-theme', resolvedTheme.value);
}

function applyLocale() {
  resolvedLocale.value = resolveLocale(localePreference.value);
  document.documentElement.setAttribute('lang', resolvedLocale.value);
}

/** Wire the preferences to the document; called once from main.ts. */
export function installPreferences() {
  applyTheme();
  applyLocale();
  watch(themePreference, (value) => {
    write(THEME_KEY, value);
    applyTheme();
  });
  watch(localePreference, (value) => {
    write(LOCALE_KEY, value);
    applyLocale();
  });
  globalThis
    .matchMedia?.('(prefers-color-scheme: light)')
    .addEventListener('change', () => {
      if (themePreference.value === 'system') applyTheme();
    });
  window.addEventListener('languagechange', () => {
    if (localePreference.value === 'system') applyLocale();
  });
}
