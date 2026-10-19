import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { expect, test } from '@playwright/test';
import { ADMIN_KUBECONFIG } from '../playwright.it.config';

const ROOT = resolve(import.meta.dirname, '../../..');
const NS = 'it-e2e-login';

function admin(args: string[], input?: string) {
  return execFileSync('kubectl', ['--kubeconfig', ADMIN_KUBECONFIG, ...args], {
    input,
    encoding: 'utf8',
  });
}

function persona(name: string): { email: string; password: string } {
  const text = readFileSync(resolve(ROOT, '.it/personas.yaml'), 'utf8');
  const block = text
    .split(/\n {2}(?=\w+:\n)/)
    .find((b: string) => b.startsWith(`${name}:`));
  const field = (key: string) =>
    block?.match(new RegExp(`\\n\\s+${key}: (\\S+)`))?.[1];
  const email = field('email');
  const password = field('password');
  if (!email || !password)
    throw new Error(`no persona ${name} in .it/personas.yaml`);
  return { email, password };
}

test.beforeAll(() => {
  const busybox = readFileSync(resolve(ROOT, '.it/images.lock'), 'utf8').match(
    /^busybox\s+(\S+)/m,
  )?.[1];
  const fixture = readFileSync(
    resolve(ROOT, 'tests/integration/fixtures/smoke.yaml'),
    'utf8',
  )
    .replaceAll('__NS__', NS)
    .replaceAll('__BUSYBOX__', busybox ?? '');
  admin(['delete', 'namespace', NS, '--ignore-not-found', '--wait']);
  admin(['create', 'namespace', NS]);
  admin(['apply', '-f', '-'], fixture);
  // Listed by the API once the controller reconciled it.
  admin([
    '-n',
    NS,
    'wait',
    'proxykubeapi/smoke',
    '--for=jsonpath={.status.exposed}=true',
    '--timeout=120s',
  ]);
});

test.afterAll(() => {
  if (!process.env.IT_KEEP) admin(['delete', 'namespace', NS, '--wait=false']);
});

/** RFC 0009 §6.P point 7: the whole login on the real stack, nothing intercepted. */
test('full login', async ({ page }) => {
  const reader = persona('reader');
  const dex: string[] = [];
  page.on('request', (request) => {
    const url = new URL(request.url());
    if (url.hostname === 'dex.k8s.localhost' && url.pathname === '/auth')
      dex.push(url.href);
  });

  await page.goto('/');
  await page
    .getByRole('button', { name: /log in|se connecter/i })
    .first()
    .click();

  await page.waitForURL(/dex\.k8s\.localhost/);
  await page.locator('#login').fill(reader.email);
  await page.locator('#password').fill(reader.password);
  await page.locator('#submit-login').click();

  await page.waitForURL((url) => url.hostname === 'proxyauth.k8s.localhost');
  await expect(page.getByText('smoke').first()).toBeVisible();
  expect(dex.length).toBeGreaterThan(0);
});
