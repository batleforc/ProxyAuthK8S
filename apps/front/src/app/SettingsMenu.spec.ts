import { describe, expect, it, vi } from 'vitest';
import { mount } from '@vue/test-utils';
import { nextTick } from 'vue';
import { i18n } from '../i18n';
import SettingsMenu from './SettingsMenu.vue';

describe('SettingsMenu', () => {
  it('drops its document listeners when unmounted while open', async () => {
    const wrapper = mount(SettingsMenu, {
      global: { plugins: [i18n] },
      attachTo: document.body,
    });
    await wrapper.get('button[aria-controls]').trigger('click');
    await nextTick();
    const removed = vi.spyOn(document, 'removeEventListener');
    wrapper.unmount();
    const events = removed.mock.calls.map(([type]) => type);
    expect(events).toContain('keydown');
    expect(events).toContain('pointerdown');
    removed.mockRestore();
  });
});
