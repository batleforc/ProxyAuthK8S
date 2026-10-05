import { defineStore } from 'pinia';
import {
  callbackLogin,
  CallbackModel,
  clusterLogin,
  getAllVisibleCluster,
  VisibleCluster,
} from '@proxy-auth-k8s/front-api';
import { useAuthStore } from './auth.ts';
import { useToast } from 'maz-ui/composables/useToast';
import { safeRedirectTarget } from '../utils/redirect.ts';

export const useClustersStore = defineStore('clusters', {
  state: () => ({
    clusters: [] as Array<VisibleCluster>,
    inited: false,
    callBack: {
      ns: '',
      cluster: '',
      retour: {} as CallbackModel,
    },
  }),
  getters: {
    getClusters(): Array<VisibleCluster> {
      return this.clusters;
    },
    isInited(): boolean {
      return this.inited;
    },
  },
  actions: {
    async fetchClusters(toast = useToast()) {
      const authStore = useAuthStore();
      return await getAllVisibleCluster({
        headers: {
          Authorization: `Bearer ${authStore.user?.access_token}`,
        },
      }).then((response) => {
        if (response.status === 200 && response.data) {
          this.clusters = response.data.clusters;
        } else if (response.status === 200 && response.data === undefined) {
          this.clusters = [];
          console.error('No cluster data received');
          toast.error('No cluster data received from server', {
            timeout: 5000,
          });
        } else if (response.status === 401) {
          console.error('Unauthorized access when fetching clusters');
          toast.error('Unauthorized access. Please log in again.', {
            timeout: 2000,
          });
          setTimeout(() => {
            //authSore.logIn();
          }, 2000);
          this.clusters = [];
        } else {
          toast.warning(`Unexpected response: ${response.status}`, {
            timeout: 5000,
          });
          console.error(`Unexpected response status: ${response.status}`);
          this.clusters = [];
        }
        this.inited = true;
      });
    },
    async redirectToLogin(ns: string, cluster: string) {
      const authStore = useAuthStore();
      return await clusterLogin({
        path: { ns, cluster },
        headers: {
          Authorization: `Bearer ${authStore.user?.access_token}`,
          'x-front-callback': 'true',
          'x-kubectl-callback': null,
        },
      }).then((response) => {
        if (response.status === 200 && response.data) {
          const target = safeRedirectTarget(response.data);
          if (target) {
            window.location.href = target;
          } else {
            console.error('Invalid URL received for cluster login redirect');
          }
        } else if (response.status === 401) {
          console.error(
            'Unauthorized access when redirecting to cluster login'
          );
        }
      });
    },
    async callBackFromCluster(toast = useToast()) {
      await this.router.isReady();
      const ns = this.router.currentRoute.value.params.ns as string;
      const cluster = this.router.currentRoute.value.params.cluster as string;
      const code = this.router.currentRoute.value.query.code as string;
      const state = this.router.currentRoute.value.query.state as string;
      if (!ns || !cluster || !code || !state) {
        toast.error('Missing parameters in callback URL', { timeout: 5000 });
        console.error('Missing parameters in callback URL');
        setTimeout(() => {
          this.router.push({ name: 'home' });
        }, 2000);
        return;
      }
      this.callBack.ns = ns;
      this.callBack.cluster = cluster;
      return await callbackLogin({
        headers: {
          'x-front-callback': 'true',
          'x-kubectl-callback': null,
        },
        path: {
          ns,
          cluster,
        },
        query: {
          code,
          state,
        },
      })
        .then((response) => {
          if (response.status === 200 && response.data) {
            this.callBack.retour = response.data;
            toast.success('Successfully authenticated with the cluster', {
              timeout: 3000,
            });
          } else if (response.status === 401) {
            toast.error('Unauthorized access during callback login', {
              timeout: 5000,
            });
            console.error('Unauthorized access during callback login');
            setTimeout(() => {
              this.router.push({ name: 'home' });
            }, 2000);
          }
        })
        .catch((error) => {
          toast.error(`Error during callback login: ${error}`, {
            timeout: 5000,
          });
          console.error('Error during callback login:', error);
          setTimeout(() => {
            this.router.push({ name: 'home' });
          }, 2000);
        });
    },
  },
});
