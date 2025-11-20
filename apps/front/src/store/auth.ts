import { defineStore } from 'pinia';
import { userManager } from '../oidc/config.ts';
import { User } from 'oidc-client-ts';

export const useAuthStore = defineStore('auth', {
  state: () => ({
    inited: false,
    user: null as User | null,
  }),
  getters: {
    isAuthenticated(): boolean {
      return this.user !== null && !this.user.expired;
    },
    getUserProfile(): User | null {
      return this.user;
    },
  },
  actions: {
    async init() {
      this.inited = true;
      let user = await this.getUser().then((user) => {
        if (user && !user.expired) {
          console.log('User is logged in', user);
        } else if (user && user.expired) {
          userManager.signinSilent();
        } else if (
          window.location.pathname !== '/auth/callback' &&
          this.router.currentRoute.value.meta.requiresAuth
        ) {
          this.logIn();
        } else if (window.location.pathname === '/auth/callback') {
          console.log('On callback route, not redirecting to login');
          return this.callback().then((user) => {
            this.router.push('/');
            console.log('User logged in after callback', user);
            return user;
          });
        }
        return user;
      });
      this.user = user;

      this.router.beforeEach(async (to, from) => {
        if (to.meta.requiresAuth && (!this.user || this.user.expired)) {
          console.log('Route requires auth, redirecting to login');
          this.logIn();
        }
      });
    },
    logIn() {
      try {
        userManager.signinRedirect();
      } catch (error) {
        console.error('Error during sign in:', error);
        // Handle error appropriately, e.g., show notification to user
      }
    },
    logOut() {
      try {
        userManager.signoutRedirect();
      } catch (error) {
        console.error('Error during sign out:', error);
        // Handle error appropriately, e.g., show notification to user
      }
    },
    callback() {
      return userManager.signinRedirectCallback();
    },
    getUser() {
      return userManager.getUser();
    },
  },
});
