import { describe, expect, it } from 'vitest';
import { mount } from '@vue/test-utils';
import KubeconfigBlock from './KubeconfigBlock.vue';
import { stubDirectives } from '../../test/mountHelpers';

function mountBlock(framed = false) {
  return mount(KubeconfigBlock, {
    props: {
      content: 'apiVersion: v1\nkind: Config',
      downloadLabel: 'Télécharger kubeconfig',
      copyLabel: 'Copier kubeconfig',
      framed,
    },
    global: { directives: stubDirectives },
  });
}

describe('KubeconfigBlock', () => {
  it('renders the YAML and both action labels', () => {
    const wrapper = mountBlock();

    expect(wrapper.find('code.hljs.yaml').text()).toBe('apiVersion: v1\nkind: Config');
    expect(wrapper.find('.download-button').text()).toBe('Télécharger kubeconfig');
    expect(wrapper.find('.copy-kubeconfig-button').text()).toBe('Copier kubeconfig');
    expect(wrapper.find('.kubeconfig-field').classes()).not.toContain('kubeconfig-field--framed');
  });

  it('emits download and copy from the buttons', async () => {
    const wrapper = mountBlock();

    await wrapper.find('.download-button').trigger('click');
    await wrapper.find('.copy-kubeconfig-button').trigger('click');

    expect(wrapper.emitted('download')).toHaveLength(1);
    expect(wrapper.emitted('copy')).toHaveLength(1);
  });

  it('adds the framed variant class', () => {
    expect(mountBlock(true).find('.kubeconfig-field').classes()).toContain('kubeconfig-field--framed');
  });
});
