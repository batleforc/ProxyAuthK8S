import { beforeEach, describe, expect, it, vi } from 'vitest';
import { mount } from '@vue/test-utils';

async function load() {
  vi.resetModules();
  const prefs = await import('../composables/usePreferences');
  const { i18n } = await import('../i18n');
  const LocaleToggle = (await import('./LocaleToggle.vue')).default;
  return { prefs, i18n, LocaleToggle };
}

describe('LocaleToggle', () => {
  beforeEach(() => localStorage.clear());

  it('resolves System from the browser language, with an English fallback', async () => {
    const { prefs } = await load();
    vi.spyOn(navigator, 'language', 'get').mockReturnValue('fr-FR');
    expect(prefs.resolveLocale('system')).toBe('fr');
    vi.spyOn(navigator, 'language', 'get').mockReturnValue('de-DE');
    expect(prefs.resolveLocale('system')).toBe('en');
  });

  it('stores the choice and sets the document language', async () => {
    const { prefs, i18n, LocaleToggle } = await load();
    prefs.installPreferences();
    const wrapper = mount(LocaleToggle, { global: { plugins: [i18n] } });
    await wrapper.findAll('button')[2].trigger('click');

    expect(localStorage.getItem('pak.locale')).toBe('fr');
    expect(document.documentElement.getAttribute('lang')).toBe('fr');
    expect(prefs.resolvedLocale.value).toBe('fr');
  });

  it('ignores a stored value outside the list', async () => {
    localStorage.setItem('pak.locale', 'de');
    const { prefs } = await load();
    expect(prefs.localePreference.value).toBe('system');
  });
});
