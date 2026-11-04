import { test, expect, Page } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { Bridge, startBridge } from '../fixtures';
import { createVault } from '../helpers';

let bridge: Bridge;
test.afterEach(async () => bridge?.stop());

const current = JSON.parse(readFileSync(join(__dirname, '..', '..', 'ui', 'package.json'), 'utf8')).version as string;
const GH = 'https://api.github.com/repos/dest1256/d256-Shell/releases/latest';

/** Pretend to be GitHub: `tag` is the latest release; null = the network is down. Returns the request counter. */
async function mockGithub(page: Page, tag: string | null): Promise<{ n: number }> {
  const hits = { n: 0 };
  await page.route(GH, async (route) => {
    hits.n++;
    if (tag === null) return route.abort('failed');
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      headers: { 'access-control-allow-origin': '*' },
      body: JSON.stringify({ tag_name: tag, html_url: `https://github.com/dest1256/d256-Shell/releases/tag/${tag}` }),
    });
  });
  return hits;
}

test('updates: a newer release is announced at start (never installed), the dismissal is remembered', async ({ page }) => {
  const hits = await mockGithub(page, 'v99.0.0');
  bridge = await startBridge();
  await page.goto(bridge.url);
  await createVault(page);
  const banner = page.getByTestId('update-banner');
  await expect(banner).toBeVisible();
  await expect(banner).toContainText('99.0.0');
  await page.screenshot({ path: 'screenshots/81-update-banner.png' });
  expect(hits.n).toBe(1);
  // the link leads to the release page of this repository only
  const [popup] = await Promise.all([page.waitForEvent('popup'), page.getByTestId('update-release').click()]);
  expect(popup.url()).toContain('github.com/dest1256/d256-Shell/releases/tag/v99.0.0');
  await popup.close();
  await page.getByTestId('update-hide').click();
  await expect(banner).toBeHidden();
  // asked once a day: a reload does not ask again, and does not show the dismissed version
  await page.reload();
  await expect(page.getByTestId('rail-sessions')).toBeVisible();
  await page.waitForTimeout(800);
  expect(hits.n).toBe(1);
  await expect(banner).toBeHidden();
});

test('updates: the same or an older version shows nothing; "Check now" in Settings says so', async ({ page }) => {
  const hits = await mockGithub(page, 'v' + current);
  bridge = await startBridge();
  await page.goto(bridge.url);
  await createVault(page);
  await expect(page.getByTestId('rail-sessions')).toBeVisible();
  await expect.poll(() => hits.n).toBe(1);
  await expect(page.getByTestId('update-banner')).toBeHidden();
  await page.getByRole('button', { name: 'Settings' }).click();
  await expect(page.getByTestId('set-updates')).toBeChecked();
  await page.getByTestId('update-check').click();
  await expect(page.getByTestId('update-result')).toContainText(`You have the latest version (${current})`);
  expect(hits.n).toBe(2);
  // an older release is "current" too
  await page.unroute(GH);
  await mockGithub(page, 'v0.0.1');
  await page.getByTestId('update-check').click();
  await expect(page.getByTestId('update-result')).toContainText('latest version');
});

test('updates: no network is not an error at start; "Check now" explains; switching it off sends nothing', async ({ page }) => {
  const hits = await mockGithub(page, null);
  bridge = await startBridge();
  await page.goto(bridge.url);
  await createVault(page);
  await expect.poll(() => hits.n).toBe(1);
  await expect(page.getByTestId('update-banner')).toBeHidden();
  await expect(page.getByTestId('confirm')).toHaveCount(0);
  await page.getByRole('button', { name: 'Settings' }).click();
  await page.getByTestId('update-check').click();
  await expect(page.getByTestId('update-result')).toContainText('Could not check for updates');
  // switched off in Settings: nothing is requested at the next start
  await page.getByTestId('set-updates').uncheck();
  await page.getByTestId('settings-save').click();
  await page.evaluate(() => localStorage.removeItem('d256shell.update-checked'));
  const before = hits.n;
  await page.reload();
  await expect(page.getByTestId('rail-sessions')).toBeVisible();
  await page.waitForTimeout(1000);
  expect(hits.n).toBe(before);
});
