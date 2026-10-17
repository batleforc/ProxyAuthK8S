import { defineConfig, devices } from '@playwright/test';

// The built front, served by `vite preview`, with an issuer and an API that
// only the tests answer (page.route): no IdP, no backend needed.
const PORT = 4300;
const OUT = '../../dist/e2e-front';
export const ORIGIN = `http://127.0.0.1:${PORT}`;
export const ISSUER = `${ORIGIN}/idp`;

export default defineConfig({
  testDir: './e2e',
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  reporter: process.env.CI ? 'github' : 'list',
  use: { baseURL: ORIGIN, trace: 'retain-on-failure' },
  projects: [
    { name: 'chromium', use: { ...devices['Desktop Chrome'] } },
    { name: 'firefox', use: { ...devices['Desktop Firefox'] } },
  ],
  webServer: {
    command: `vite build --outDir ${OUT} --emptyOutDir && vite preview --outDir ${OUT} --host 127.0.0.1 --port ${PORT} --strictPort`,
    url: ORIGIN,
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
    env: {
      VITE_API_BASE_URL: ORIGIN,
      VITE_OIDC_ISSUER_URL: ISSUER,
      VITE_OIDC_CLIENT_ID: 'front',
      VITE_OIDC_SCOPE: 'openid profile',
      VITE_OIDC_SILENT_REFRESH: 'false',
    },
  },
});
