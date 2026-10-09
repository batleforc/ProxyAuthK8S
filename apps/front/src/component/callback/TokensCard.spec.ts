import { beforeEach, describe, expect, it, vi } from 'vitest';
import { flushPromises, mount } from '@vue/test-utils';

const toast = vi.hoisted(() => ({ success: vi.fn(), error: vi.fn() }));
vi.mock('maz-ui/composables/useToast', () => ({ useToast: () => toast }));

import TokensCard from './TokensCard.vue';

const tokens = {
  access_token: 'ACCESS',
  id_token: 'ID',
  refresh_token: 'REFRESH',
  subject: 'jane',
  cluster_url: 'https://proxy.example.com/clusters/team/prod',
};

describe('TokensCard', () => {
  const writeText = vi.fn().mockResolvedValue(undefined);

  beforeEach(() => {
    vi.clearAllMocks();
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
  });

  it('shows the access, id and refresh tokens', () => {
    const wrapper = mount(TokensCard, { props: { tokens } });

    expect(wrapper.findAll('.token-label').map((l) => l.text())).toEqual([
      'Access Token:',
      'ID Token:',
      'Refresh Token:',
    ]);
    expect(wrapper.findAll('textarea').map((t) => t.element.value)).toEqual(['ACCESS', 'ID', 'REFRESH']);
  });

  it('hides the refresh token when the provider returned none', () => {
    const wrapper = mount(TokensCard, { props: { tokens: { ...tokens, refresh_token: '' } } });

    expect(wrapper.findAll('.token-section')).toHaveLength(2);
    expect(wrapper.find('#refresh-token').exists()).toBe(false);
  });

  it('copies the matching token', async () => {
    const wrapper = mount(TokensCard, { props: { tokens } });

    await wrapper.findAll('.copy-button')[1].trigger('click');
    await flushPromises();

    expect(writeText).toHaveBeenCalledWith('ID');
    expect(toast.success).toHaveBeenCalledWith('ID Token copié dans le presse-papiers !');
  });
});
