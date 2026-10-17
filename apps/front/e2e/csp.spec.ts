import { expect, test } from '@playwright/test';

/**
 * RFC 0002 §6.P points 8 and 9, against the built image (not vite preview):
 *   podman run -p 14201:4200 -e PROXYAUTHK8S_FRONT_API_BASE_URL=https://… image
 *   FRONT_IMAGE_URL=http://127.0.0.1:14201 pnpm exec playwright test csp
 * Skipped unless FRONT_IMAGE_URL is set (`task ui:csp` sets it).
 */
const IMAGE = process.env.FRONT_IMAGE_URL;
test.skip(!IMAGE, 'FRONT_IMAGE_URL is not set');

test('the image serves its CSP, and the app runs under it', async ({
  page,
}) => {
  const violations: string[] = [];
  page.on('console', (message) => {
    // Chromium logs "Content Security Policy", Firefox "Content-Security-Policy".
    if (/Content[- ]Security[- ]Policy/i.test(message.text()))
      violations.push(message.text());
  });
  const response = await page.goto(`${IMAGE}/about`);
  const headers = response?.headers() ?? {};
  expect(headers['content-security-policy']).toContain(
    "script-src 'self' 'sha256-",
  );
  expect(headers['x-content-type-options']).toBe('nosniff');
  expect(headers['referrer-policy']).toBe('no-referrer');

  await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
  // The inline theme script ran: its hash is allowed.
  expect(
    await page.evaluate(() => document.documentElement.dataset.theme),
  ).toMatch(/^(light|dark)$/);
  expect(violations).toEqual([]);

  // reka-ui positions the settings popover with styles set through the
  // CSSOM, which `style-src 'self'` allows: it opens with no violation.
  await page.getByRole('button', { name: /réglages|settings/i }).click();
  await expect(page.getByRole('group').first()).toBeVisible();
  await page.keyboard.press('Escape');
  expect(violations).toEqual([]);

  // An injected inline script is refused, and the app keeps working.
  await page.evaluate(() => {
    const script = document.createElement('script');
    script.textContent = 'window.injected = true';
    document.body.append(script);
  });
  expect(
    await page.evaluate(() => (window as { injected?: boolean }).injected),
  ).toBeUndefined();
  expect(violations.some((text) => /script/i.test(text))).toBe(true);
  await page
    .getByRole('link', { name: /accueil|home/i })
    .first()
    .click();
  await expect(page).toHaveURL(`${IMAGE}/`);
});
