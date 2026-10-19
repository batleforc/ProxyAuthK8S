import { beforeEach, describe, expect, it, vi } from 'vitest';
import { mount } from '@vue/test-utils';

async function load() {
  vi.resetModules();
  const prefs = await import('../composables/usePreferences');
  const { i18n } = await import('../i18n');
  const ThemeToggle = (await import('./ThemeToggle.vue')).default;
  return { prefs, i18n, ThemeToggle };
}

describe('ThemeToggle (RFC 0002 §4.1, T5)', () => {
  beforeEach(() => {
    localStorage.clear();
    document.documentElement.removeAttribute('data-theme');
  });

  it('stores the preference, not the resolved value, and applies it', async () => {
    const { prefs, i18n, ThemeToggle } = await load();
    prefs.installPreferences();
    const wrapper = mount(ThemeToggle, { global: { plugins: [i18n] } });

    const buttons = wrapper.findAll('button');
    expect(buttons.map((b) => b.attributes('aria-pressed'))).toEqual([
      'true',
      'false',
      'false',
    ]);
    await buttons[1].trigger('click');

    expect(localStorage.getItem('pak.theme')).toBe('light');
    expect(document.documentElement.getAttribute('data-theme')).toBe('light');
    expect(wrapper.findAll('button')[1].attributes('aria-pressed')).toBe(
      'true',
    );
    // The System note only shows while the preference is System.
    expect(wrapper.text()).not.toContain('System follows');
  });

  it('ignores a stored value outside the list', async () => {
    localStorage.setItem('pak.theme', 'url(javascript:alert(1))');
    const { prefs } = await load();
    expect(prefs.themePreference.value).toBe('system');
  });

  it('survives an unavailable storage', async () => {
    const spy = vi
      .spyOn(Storage.prototype, 'getItem')
      .mockImplementation(() => {
        throw new Error('private mode');
      });
    const { prefs } = await load();
    expect(prefs.themePreference.value).toBe('system');
    spy.mockRestore();
  });
});
