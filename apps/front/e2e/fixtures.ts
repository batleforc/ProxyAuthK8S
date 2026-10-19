import type { Page } from '@playwright/test';
import { ISSUER } from '../playwright.config';

/** The clusters the mocked API lists, and the page set-up the suites share. */
export const CLUSTERS = [
  {
    name: 'prod',
    namespace: 'team',
    enabled: true,
    sso_enabled: true,
    is_reachable: true,
  },
  {
    name: 'stopped',
    namespace: 'team',
    enabled: true,
    sso_enabled: true,
    is_reachable: false,
  },
  {
    name: 'off',
    namespace: 'team',
    enabled: false,
    sso_enabled: false,
    is_reachable: null,
  },
  {
    name: 'dev',
    namespace: 'team',
    enabled: true,
    sso_enabled: false,
    is_reachable: null,
  },
];

export async function setUp(
  page: Page,
  opts: { theme: 'light' | 'dark'; loggedIn: boolean; locale?: 'en' | 'fr' },
) {
  const thirdParty: string[] = [];
  page.on('request', (request) => {
    const url = new URL(request.url());
    if (url.hostname !== '127.0.0.1') thirdParty.push(request.url());
  });
  await page.addInitScript(
    ({ theme, loggedIn, issuer, locale }) => {
      localStorage.setItem('pak.theme', theme);
      localStorage.setItem('pak.locale', locale);
      if (loggedIn) {
        sessionStorage.setItem(
          `oidc.user:${issuer}:front`,
          JSON.stringify({
            access_token: 'test-token',
            token_type: 'Bearer',
            scope: 'openid profile',
            profile: {
              sub: 'dev-user',
              name: 'Dev User',
              preferred_username: 'dev-user',
            },
            expires_at: Math.floor(Date.now() / 1000) + 3600,
          }),
        );
      }
    },
    { locale: 'fr', ...opts, issuer: ISSUER },
  );
  await page.route('**/api/v1/clusters', (route) =>
    route.fulfill({ json: { clusters: CLUSTERS } }),
  );
  await page.route('**/clusters/team/prod/auth/callback**', (route) =>
    route.fulfill({
      json: {
        access_token: 'ACCESS',
        id_token: 'ID',
        refresh_token: 'REFRESH',
        subject: 'dev-user',
        cluster_url: 'https://proxy.example.com/clusters/team/prod',
      },
    }),
  );
  // The issuer is never reachable: login redirects fail closed in the test.
  await page.route(`${ISSUER}/**`, (route) => route.abort());
  return thirdParty;
}
