import { defineStore } from 'pinia';
import { userManager } from '../oidc/config';

export const useAuthStore = defineStore('auth', {
  state: () => ({
    inited: false,
  }),
  actions: {
    async init() {
      this.inited = true;
      let user = await userManager.getUser().then((user) => {
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
          this.callback().then(() => {
            this.router.push('/');
          });
        }
        return user;
      });

      this.router.beforeEach(async (to, from) => {
        if (to.meta.requiresAuth && (!user || user.expired)) {
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
