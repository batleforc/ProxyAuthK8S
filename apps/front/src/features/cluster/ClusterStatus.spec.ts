import { describe, expect, it } from 'vitest';
import { mount } from '@vue/test-utils';
import { i18n } from '../../i18n';
import ClusterStatus from './ClusterStatus.vue';
import { clusterStatus } from './status';

const base = {
  name: 'prod',
  namespace: 'team',
  enabled: true,
  sso_enabled: true,
  is_reachable: true,
} as never;

function render(status: 'available' | 'unreachable' | 'disabled' | 'unknown') {
  i18n.global.locale.value = 'fr';
  return mount(ClusterStatus, {
    props: { status },
    global: { plugins: [i18n] },
  });
}

describe('ClusterStatus: three channels (RFC 0002 §4.2)', () => {
  it.each([
    // status, word, matrix cells (0 = none), lit cells, hue class
    ['available', 'Disponible', 9, 9, 'text-foreground'],
    ['unreachable', 'Injoignable', 4, 4, 'text-destructive'],
    ['unknown', 'État inconnu', 4, 3, 'text-copper'],
  ] as const)('%s: pattern, word and hue', (status, word, cells, lit, hue) => {
    const wrapper = render(status);
    const matrix = wrapper.find('[data-testid="resolution-matrix"]');
    expect(matrix.attributes('aria-hidden')).toBe('true');
    expect(matrix.findAll('i')).toHaveLength(cells);
    expect(matrix.findAll('i.opacity-100')).toHaveLength(lit);
    expect(wrapper.text()).toBe(word);
    expect(wrapper.classes()).toContain(hue);
  });

  it('disabled: no pattern, a dotted rule, dim ink', () => {
    const wrapper = render('disabled');
    expect(wrapper.find('[data-testid="resolution-matrix"]').exists()).toBe(
      false,
    );
    expect(wrapper.classes()).toEqual(
      expect.arrayContaining(['border-dashed', 'text-muted-foreground']),
    );
    expect(wrapper.text()).toBe('Désactivé');
  });

  it('unknown never reads as healthy', () => {
    expect(render('unknown').classes()).not.toContain('text-foreground');
  });

  it('derives the status like the old view did', () => {
    expect(
      clusterStatus({ ...(base as object), enabled: false } as never),
    ).toBe('disabled');
    expect(
      clusterStatus({ ...(base as object), is_reachable: false } as never),
    ).toBe('unreachable');
    expect(clusterStatus(base)).toBe('available');
    expect(
      clusterStatus({ ...(base as object), is_reachable: null } as never),
    ).toBe('unknown');
  });
});
