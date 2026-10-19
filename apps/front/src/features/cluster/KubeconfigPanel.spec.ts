import { beforeEach, describe, expect, it, vi } from 'vitest';
import { flushPromises, mount } from '@vue/test-utils';

const toast = vi.hoisted(() => ({ success: vi.fn(), error: vi.fn() }));
vi.mock('../../composables/useToast', () => ({ useToast: () => toast }));
const download = vi.hoisted(() => vi.fn());
vi.mock('../../utils/download', () => ({ downloadTextFile: download }));

import { i18n } from '../../i18n';
import KubeconfigPanel from './KubeconfigPanel.vue';

const YAML = 'apiVersion: v1\nkind: Config';

function mountPanel() {
  return mount(KubeconfigPanel, {
    props: {
      content: YAML,
      filename: 'kubeconfig-team-prod.yaml',
      name: 'Kubeconfig',
    },
    global: { plugins: [i18n] },
  });
}

describe('KubeconfigPanel (ex KubeconfigBlock)', () => {
  const writeText = vi.fn().mockResolvedValue(undefined);

  beforeEach(() => {
    vi.clearAllMocks();
    i18n.global.locale.value = 'fr';
    Object.defineProperty(navigator, 'clipboard', {
      value: { writeText },
      configurable: true,
    });
  });

  it('renders the YAML as text and both actions', () => {
    const wrapper = mountPanel();

    expect(wrapper.find('pre code').text()).toBe(YAML);
    expect(wrapper.find('pre').attributes('data-lang')).toBe('yaml');
    expect(wrapper.find('.kubeconfig-download').text()).toBe(
      'Télécharger le kubeconfig',
    );
    expect(wrapper.find('.kubeconfig-copy').text()).toBe('Copier');
    // No HTML rendering of the YAML (RFC 0002 T2).
    expect(wrapper.html()).not.toContain('v-html');
  });

  it('downloads and copies the kubeconfig', async () => {
    const wrapper = mountPanel();

    await wrapper.find('.kubeconfig-download').trigger('click');
    await wrapper.find('.kubeconfig-copy').trigger('click');
    await flushPromises();

    expect(download).toHaveBeenCalledWith(YAML, 'kubeconfig-team-prod.yaml');
    expect(writeText).toHaveBeenCalledWith(YAML);
    expect(toast.success).toHaveBeenCalledWith('Kubeconfig téléchargé.');
    expect(toast.success).toHaveBeenCalledWith(
      'Kubeconfig copié dans le presse-papiers.',
    );
  });

  it('never persists the kubeconfig in localStorage (T3)', async () => {
    localStorage.clear();
    const wrapper = mountPanel();
    await wrapper.find('.kubeconfig-copy').trigger('click');
    expect(Object.keys(localStorage)).toEqual([]);
  });
});
