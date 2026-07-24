import { UserManager, WebStorageStateStore } from 'oidc-client-ts';

const oidcConfig = {
  authority: import.meta.env.VITE_OIDC_ISSUER_URL,
  client_id: import.meta.env.VITE_OIDC_CLIENT_ID,
  redirect_uri: `${window.location.origin}/auth/callback`,
  post_logout_redirect_uri: `${window.location.origin}/`,
  response_type: 'code',
  scope: import.meta.env.VITE_OIDC_SCOPE,
  automaticSilentRenew: import.meta.env.VITE_OIDC_SILENT_REFRESH === 'true',
  loadUserInfo: true,
  // Keep tokens in sessionStorage rather than the default localStorage so they
  // are scoped to the tab and cleared on close, shrinking the window in which a
  // dependency XSS could exfiltrate live Kubernetes tokens.
  userStore: new WebStorageStateStore({ store: window.sessionStorage }),
};

export const userManager = new UserManager(oidcConfig);
