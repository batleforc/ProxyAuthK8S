import { UserManager } from 'oidc-client-ts';

const oidcConfig = {
  authority: import.meta.env.VITE_OIDC_ISSUER_URL,
  client_id: import.meta.env.VITE_OIDC_CLIENT_ID,
  redirect_uri: `${window.location.origin}/auth/callback`,
  post_logout_redirect_uri: `${window.location.origin}/`,
  response_type: 'code',
  scope: import.meta.env.VITE_OIDC_SCOPE,
  automaticSilentRenew: import.meta.env.VITE_OIDC_SILENT_REFRESH === 'true',
  loadUserInfo: true,
};

export const userManager = new UserManager(oidcConfig);
