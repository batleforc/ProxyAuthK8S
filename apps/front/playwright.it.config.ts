import { X509Certificate, createHash } from 'node:crypto';
import { existsSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { defineConfig, devices } from '@playwright/test';

// The real stack of the k3d bench (RFC 0009 §6.4): no page.route, a real
// Dex login. `task it:up` first; `task it:test` runs this suite.
const RUN = resolve(
  import.meta.dirname,
  '../..',
  process.env.IT_DIR ?? '.it/run',
);
const PORT = process.env.IT_HTTPS_PORT ?? '8443';
export const ORIGIN = `https://proxyauth.k8s.localhost:${PORT}`;
export const ADMIN_KUBECONFIG = resolve(RUN, 'admin.kubeconfig');

// Chromium reads no CA from the environment: accept the run certificate's
// key, and only that key, instead of trusting a CA.
function spki(): string {
  const cert = resolve(RUN, 'tls.crt');
  if (!existsSync(cert))
    throw new Error(`no bench at ${RUN}: run \`task it:up\` first`);
  const der = new X509Certificate(readFileSync(cert)).publicKey.export({
    type: 'spki',
    format: 'der',
  });
  return createHash('sha256').update(der).digest('base64');
}

export default defineConfig({
  testDir: './e2e-it',
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: 0,
  reporter: process.env.CI ? [['github'], ['html', { open: 'never' }]] : 'list',
  use: {
    baseURL: ORIGIN,
    ignoreHTTPSErrors: false,
    trace: 'retain-on-failure',
  },
  projects: [
    {
      name: 'chromium',
      use: {
        ...devices['Desktop Chrome'],
        launchOptions: {
          args: [
            `--ignore-certificate-errors-spki-list=${spki()}`,
            '--host-resolver-rules=MAP proxyauth.k8s.localhost 127.0.0.1, MAP dex.k8s.localhost 127.0.0.1',
          ],
        },
      },
    },
  ],
});
