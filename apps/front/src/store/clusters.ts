import { defineStore } from 'pinia';
import {
  getAllVisibleCluster,
  VisibleCluster,
} from '@proxy-auth-k8s/front-api';
import { useAuthStore } from './auth.ts';
import { useToast } from 'maz-ui/composables/useToast';

const toast = useToast();

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
    fetchClusters() {
      const authSore = useAuthStore();
      return getAllVisibleCluster({
        headers: {
          Authorization: `Bearer ${authSore.user?.access_token}`,
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
            authSore.logIn();
          }, 2000);
          this.clusters = [];
        }
        this.clusters = [];
        this.inited = true;
      });
    },
  },
});
