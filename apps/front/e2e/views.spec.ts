import { expect, test } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { ISSUER } from '../playwright.config';
import { setUp } from './fixtures';

/**
 * RFC 0002 §6.P in Chromium and Firefox: axe (contrast included) in both themes, no
 * horizontal scroll at 360px, no third-party request, no theme flash, and the
 * route guard still in front of /cli.
 */
const VIEWS = [
  { path: '/', loggedIn: false, ready: 'ProxyAuthK8S' },
  { path: '/about', loggedIn: false, ready: 'À propos' },
  // `/auth/callback` is finished by store/auth.ts during start-up, which then
  // navigates home: its view never shows on its own.
  { path: '/', loggedIn: true, ready: 'stopped' },
  { path: '/cli', loggedIn: true, ready: 'Plugin kubectl' },
  {
    path: '/clusters/team/dev/nosso',
    loggedIn: true,
    ready: 'Configuration Kubernetes',
  },
  {
    path: '/auth/callback/team/prod?code=c&state=s',
    loggedIn: true,
    ready: 'Tokens',
  },
];

for (const theme of ['light', 'dark'] as const) {
  for (const view of VIEWS) {
    test(`${view.path} (${view.loggedIn ? 'logged in' : 'logged out'}, ${theme})`, async ({
      page,
    }) => {
      const thirdParty = await setUp(page, { theme, loggedIn: view.loggedIn });
      await page.goto(view.path);
      await expect(page.getByText(view.ready).first()).toBeVisible();
      // Stop the notices in flight from covering what axe measures.
      await page.waitForTimeout(100);

      const results = await new AxeBuilder({ page })
        .withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'])
        .analyze();
      const serious = results.violations.filter((v) =>
        ['serious', 'critical'].includes(v.impact ?? ''),
      );
      expect(
        serious.map(
          (v) => `${v.id}: ${v.nodes.map((n) => n.target).join(' ')}`,
        ),
      ).toEqual([]);

      await page.setViewportSize({ width: 360, height: 800 });
      const overflow = await page.evaluate(
        () => document.documentElement.scrollWidth - window.innerWidth,
      );
      expect(overflow).toBeLessThanOrEqual(0);
      expect(thirdParty).toEqual([]);
    });
  }
}

test('the stored theme is applied before the first paint', async ({ page }) => {
  await setUp(page, { theme: 'light', loggedIn: false });
  await page.addInitScript(() => {
    document.addEventListener('DOMContentLoaded', () => {
      (window as unknown as { themeAtDcl: string | null }).themeAtDcl =
        document.documentElement.getAttribute('data-theme');
    });
  });
  await page.goto('/');
  expect(
    await page.evaluate(
      () => (window as unknown as { themeAtDcl: string }).themeAtDcl,
    ),
  ).toBe('light');
});

test('only same-origin fonts are requested', async ({ page }) => {
  const fonts: string[] = [];
  page.on('request', (request) => {
    if (request.resourceType() === 'font')
      fonts.push(new URL(request.url()).pathname);
  });
  await setUp(page, { theme: 'dark', loggedIn: false });
  await page.goto('/');
  await expect(page.getByText('ProxyAuthK8S').first()).toBeVisible();
  expect(fonts.length).toBeGreaterThan(0);
  expect(fonts.every((path) => path.startsWith('/fonts/'))).toBe(true);
});

test('/cli logged out still goes to the login, with no SDK call', async ({
  page,
}) => {
  const api: string[] = [];
  const issuer: string[] = [];
  page.on('request', (request) => {
    if (request.url().includes('/api/')) api.push(request.url());
    if (request.url().startsWith(ISSUER)) issuer.push(request.url());
  });
  await setUp(page, { theme: 'dark', loggedIn: false });
  // The page leaves for the (refused) issuer at once: Firefox never reaches
  // its `load` event, so wait for the commit only.
  await page.goto('/cli', { waitUntil: 'commit' });
  // store/auth.ts starts the redirect: the client asks the issuer for its
  // metadata (refused here), and nothing reaches the API.
  await expect.poll(() => issuer.length).toBeGreaterThan(0);
  expect(api).toEqual([]);
});

test('keyboard: copy the kubeconfig and hear it', async ({
  page,
  browserName,
}) => {
  await setUp(page, { theme: 'dark', loggedIn: true });
  // Firefox has no clipboard permissions: a user gesture is enough there.
  if (browserName === 'chromium')
    await page
      .context()
      .grantPermissions(['clipboard-read', 'clipboard-write']);
  await page.goto('/clusters/team/dev/nosso');
  const copy = page.getByRole('button', { name: 'Copier', exact: true });
  await expect(copy).toBeVisible();
  for (
    let i = 0;
    i < 40 && !(await copy.evaluate((el) => el === document.activeElement));
    i++
  ) {
    await page.keyboard.press('Tab');
  }
  await expect(copy).toBeFocused();
  const outline = await copy.evaluate(
    (el) => getComputedStyle(el).outlineStyle,
  );
  expect(outline).toBe('solid');
  await page.keyboard.press('Enter');
  await expect(page.getByRole('status')).toContainText('copié');
});

test('the settings popover: first cell focused, Escape returns focus, outside click closes', async ({
  page,
}) => {
  await setUp(page, { theme: 'dark', loggedIn: false });
  await page.goto('/about');
  const trigger = page.getByRole('button', { name: 'Réglages' });
  await trigger.click();
  await expect(trigger).toHaveAttribute('aria-expanded', 'true');
  await expect(
    page.getByRole('button', { name: 'Système' }).first(),
  ).toBeFocused();
  await page.getByRole('button', { name: 'Clair' }).click();
  expect(
    await page.evaluate(() => document.documentElement.dataset.theme),
  ).toBe('light');
  await page.keyboard.press('Escape');
  await expect(trigger).toBeFocused();
  await expect(trigger).toHaveAttribute('aria-expanded', 'false');
  await trigger.click();
  await page.mouse.click(5, 700);
  await expect(trigger).toHaveAttribute('aria-expanded', 'false');
});
