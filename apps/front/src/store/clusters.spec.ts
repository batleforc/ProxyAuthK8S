import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';

const api = vi.hoisted(() => ({
  getAllVisibleCluster: vi.fn(),
  clusterLogin: vi.fn(),
  callbackLogin: vi.fn(),
}));
vi.mock('@proxy-auth-k8s/front-api', () => api);
vi.mock('./auth.ts', () => ({
  useAuthStore: () => ({ user: { access_token: 'front-token' } }),
}));
vi.mock('maz-ui/composables/useToast', () => ({ useToast: () => makeToast() }));

import { useClustersStore } from './clusters';

function makeToast() {
  return { error: vi.fn(), warning: vi.fn(), success: vi.fn() };
}

describe('clusters store', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    vi.spyOn(console, 'error').mockImplementation(() => undefined);
  });

  it('stores the clusters and sends the front token', async () => {
    const clusters = [{ name: 'prod', namespace: 'team-a' }];
    api.getAllVisibleCluster.mockResolvedValue({ status: 200, data: { clusters } });
    const store = useClustersStore();
    const toast = makeToast();

    await store.fetchClusters(toast as never);

    expect(store.getClusters).toEqual(clusters);
    expect(store.isInited).toBe(true);
    expect(api.getAllVisibleCluster).toHaveBeenCalledWith({
      headers: { Authorization: 'Bearer front-token' },
    });
    expect(toast.error).not.toHaveBeenCalled();
  });

  it('empties the list and warns when the body is missing', async () => {
    api.getAllVisibleCluster.mockResolvedValue({ status: 200, data: undefined });
    const store = useClustersStore();
    const toast = makeToast();

    await store.fetchClusters(toast as never);

    expect(store.getClusters).toEqual([]);
    expect(store.isInited).toBe(true);
    expect(toast.error).toHaveBeenCalledOnce();
  });

  it('empties the list on 401', async () => {
    api.getAllVisibleCluster.mockResolvedValue({ status: 401 });
    const store = useClustersStore();
    store.clusters = [{ name: 'stale' } as never];
    const toast = makeToast();

    await store.fetchClusters(toast as never);

    expect(store.getClusters).toEqual([]);
    expect(toast.error).toHaveBeenCalledOnce();
  });

  it('empties the list and warns on an unexpected status', async () => {
    api.getAllVisibleCluster.mockResolvedValue({ status: 503 });
    const store = useClustersStore();
    const toast = makeToast();

    await store.fetchClusters(toast as never);

    expect(store.getClusters).toEqual([]);
    expect(toast.warning).toHaveBeenCalledOnce();
    expect(store.isInited).toBe(true);
  });

  describe('redirectToLogin', () => {
    let location: { href: string };

    beforeEach(() => {
      location = { href: 'http://front.example.com/' };
      vi.stubGlobal('window', { location });
    });

    it('navigates to a valid http(s) login URL', async () => {
      api.clusterLogin.mockResolvedValue({ status: 200, data: 'https://idp.example.com/auth?x=1' });
      await useClustersStore().redirectToLogin('team-a', 'prod');
      expect(location.href).toBe('https://idp.example.com/auth?x=1');
      expect(api.clusterLogin).toHaveBeenCalledWith(
        expect.objectContaining({ path: { ns: 'team-a', cluster: 'prod' } }),
      );
    });

    it('refuses a javascript: URL from the API', async () => {
      api.clusterLogin.mockResolvedValue({ status: 200, data: 'javascript:alert(1)' });
      await useClustersStore().redirectToLogin('team-a', 'prod');
      expect(location.href).toBe('http://front.example.com/');
    });
  });
});
