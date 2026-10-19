import { expect, test } from '@playwright/test';

/** RFC 0002 §6.P point 1, on the image nginx serves: fonts are same-origin. */
test('fonts come from the front only', async ({ page, baseURL }) => {
  const fonts: string[] = [];
  page.on('request', (request) => {
    if (request.resourceType() === 'font') fonts.push(request.url());
  });
  await page.goto('/');
  await expect(
    page.getByRole('button', { name: /log in|se connecter/i }).first(),
  ).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
  expect(fonts.length).toBeGreaterThan(0);
  for (const font of fonts)
    expect(new URL(font).origin).toBe(new URL(baseURL ?? '').origin);
});

/**
 * RFC 0002 §6.P point 9: the image's CSP refuses an inline script injected at
 * runtime, and the app keeps working (its own theme script is hashed).
 */
test('an injected inline script is refused', async ({ page }) => {
  await page.goto('/');
  const violation = await page.evaluate(
    () =>
      new Promise<string>((resolve) => {
        document.addEventListener('securitypolicyviolation', (e) =>
          resolve(e.violatedDirective),
        );
        const script = document.createElement('script');
        script.textContent = 'window.__injected = true;';
        document.head.appendChild(script);
        setTimeout(() => resolve('none'), 2000);
      }),
  );
  expect(violation).toMatch(/^script-src/);
  expect(
    await page.evaluate(() => (window as { __injected?: boolean }).__injected),
  ).toBeUndefined();
  await expect(
    page.getByRole('button', { name: /log in|se connecter/i }).first(),
  ).toBeVisible();
});
