import { describe, expect, it } from 'vitest';
import { mount } from '@vue/test-utils';
import { LazyMazServer } from '@maz-ui/icons';
import SectionCard from './SectionCard.vue';

describe('SectionCard', () => {
  it('renders the heading with the requested tag and the default slot', () => {
    const wrapper = mount(SectionCard, {
      props: { title: 'Informations du Cluster', icon: LazyMazServer, headingTag: 'h2' },
      slots: { default: '<p class="body">content</p>' },
    });

    const heading = wrapper.find('h2.card-title');
    expect(heading.exists()).toBe(true);
    expect(heading.classes()).toContain('card-title--h2');
    expect(heading.text()).toBe('Informations du Cluster');
    expect(wrapper.find('.body').text()).toBe('content');
    expect(wrapper.classes()).toContain('section-card');
  });

  it('renders no heading without a title', () => {
    const wrapper = mount(SectionCard, { slots: { default: 'body' } });

    expect(wrapper.find('.card-title').exists()).toBe(false);
    expect(wrapper.text()).toContain('body');
  });
});
