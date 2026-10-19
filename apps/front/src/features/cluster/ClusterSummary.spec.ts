import { describe, expect, it } from 'vitest';
import { mount } from '@vue/test-utils';
import { i18n } from '../../i18n';
import ClusterSummary from './ClusterSummary.vue';

// Replaces SectionCard.spec: the card is gone, the facts are ruled rows.
describe('ClusterSummary (ex SectionCard / ClusterInfoCard)', () => {
  it('renders every fact as a term and its value', () => {
    i18n.global.locale.value = 'fr';
    const wrapper = mount(ClusterSummary, {
      props: {
        name: 'prod',
        namespace: 'team',
        url: 'https://proxy.example.com/clusters/team/prod',
        urlLabel: 'URL du proxy',
        user: 'jane',
        status: 'available',
      },
      global: { plugins: [i18n] },
    });

    expect(wrapper.findAll('dt').map((dt) => dt.text())).toEqual([
      'Cluster',
      'Namespace',
      'URL du proxy',
      'Utilisateur',
      'Statut',
    ]);
    expect(wrapper.findAll('dd').map((dd) => dd.text())).toEqual([
      'prod',
      'team',
      'https://proxy.example.com/clusters/team/prod',
      'jane',
      'Disponible',
    ]);
  });

  it('renders no status row without a status', () => {
    const wrapper = mount(ClusterSummary, {
      props: { name: 'a', namespace: 'b', url: 'c', urlLabel: 'd', user: 'e' },
      global: { plugins: [i18n] },
    });
    expect(wrapper.findAll('dt')).toHaveLength(4);
  });
});
