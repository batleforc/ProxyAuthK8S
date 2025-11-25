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
            duration: 5000,
          });
        } else if (response.status === 401) {
          console.error('Unauthorized access when fetching clusters');
          toast.error('Unauthorized access. Please log in again.', {
            duration: 2000,
          });
          setTimeout(() => {
            //authSore.logIn();
          }, 2000);
          this.clusters = [];
        } else {
          toast.warning(`Unexpected response: ${response.status}`, {
            duration: 5000,
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
        },
      }).then((response) => {
        if (response.status === 200 && response.data) {
          // Validate that response.data is a URL
          try {
            console.log('Redirecting to cluster login URL:', response.data);
            window.location.href = response.data;
          } catch (e) {
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
      let ns = this.router.currentRoute.value.params.ns as string;
      let cluster = this.router.currentRoute.value.params.cluster as string;
      let code = this.router.currentRoute.value.query.code as string;
      let state = this.router.currentRoute.value.query.state as string;
      if (!ns || !cluster || !code || !state) {
        toast.error('Missing parameters in callback URL', { duration: 5000 });
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
              duration: 3000,
            });
          } else if (response.status === 401) {
            toast.error('Unauthorized access during callback login', {
              duration: 5000,
            });
            console.error('Unauthorized access during callback login');
            setTimeout(() => {
              this.router.push({ name: 'home' });
            }, 2000);
          }
        })
        .catch((error) => {
          toast.error(`Error during callback login: ${error}`, {
            duration: 5000,
          });
          console.error('Error during callback login:', error);
          setTimeout(() => {
            this.router.push({ name: 'home' });
          }, 2000);
        });
    },
  },
});
