import { defineStore } from 'pinia';
import { userManager } from '../oidc/config.ts';
import { User } from 'oidc-client-ts';
import { useToast } from 'maz-ui/composables/useToast';

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
    isLogoutPossible(): boolean {
      return (
        this.user !== null &&
        userManager.metadataService.getEndSessionEndpoint() !== undefined
      );
    },
    isInited(): boolean {
      return this.inited;
    },
    getToken(): string | undefined {
      return this.user ? this.user.access_token : undefined;
    },
  },
  actions: {
    async init() {
      const toast = useToast();
      this.inited = true;
      await this.router.isReady();
      const user = await this.getUser().then((user) => {
        if (
          user &&
          !user.expired &&
          window.location.pathname !== '/auth/callback'
        ) {
          console.log('User is logged in', user);
          toast.success('Successfully logged in');
          return user;
        } else if (
          user &&
          user.expired &&
          userManager.settings.automaticSilentRenew
        ) {
          return userManager
            .signinSilent()
            .then((silentUser) => {
              console.log('Silent renew successful', silentUser);
              return silentUser;
            })
            .catch((err) => {
              console.error('Silent renew failed, redirecting to login', err);
              this.logIn();
              return null;
            });
        } else if (
          window.location.pathname !== '/auth/callback' &&
          this.router.currentRoute.value.meta.requiresAuth
        ) {
          toast.info('Please log in to access this page');
          this.logIn();
          return null;
        } else if (window.location.pathname === '/auth/callback') {
          console.log('On callback route, not redirecting to login');
          return this.callback()
            .then((user) => {
              this.router.push('/');
              console.log('User logged in after callback', user);
              toast.success('Successfully logged in');
              return user;
            })
            .catch((err) => {
              toast.error('Error during login callback. Please try again.');
              console.error(
                'Error handling callback (you should not be here):',
                err
              );
              this.router.push('/');
              return null;
            });
        }
        console.log('No valid user session found', {
          path: window.location.pathname,
          requiresAuth: this.router.currentRoute.value,
        });
        return null;
      });
      this.user = user;
      this.router.beforeEach(async (to) => {
        if (to.meta.requiresAuth && (!this.user || this.user.expired)) {
          console.log('Route requires auth, redirecting to login');
          toast.info('Please log in to access this page');
          this.logIn();
        }
      });
      return user;
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
      this.router.push('/').then(() => {
        window.location.reload();
      });
    },
    callback() {
      return userManager.signinRedirectCallback();
    },
    getUser() {
      return userManager.getUser();
    },
  },
});
