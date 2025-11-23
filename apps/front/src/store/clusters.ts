import { defineStore } from 'pinia';
import {
  getAllVisibleCluster,
  VisibleCluster,
} from '@proxy-auth-k8s/front-api';
import { useAuthStore } from './auth.ts';
import { useToast } from 'maz-ui/composables/useToast';

export const useClustersStore = defineStore('clusters', {
  state: () => ({
    clusters: [] as Array<VisibleCluster>,
    inited: false,
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
  },
});
