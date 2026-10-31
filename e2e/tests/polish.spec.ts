import { test, expect } from '@playwright/test';
import { Bridge, startBridge } from '../fixtures';
import { acceptHostKey, addServer, connectFirst, createVault, PASS } from '../helpers';

let bridge: Bridge;
test.afterEach(async () => bridge?.stop());

test('transfer/editor settings are saved, About shows the author and licences, tab rename', async ({ page }) => {
  bridge = await startBridge();
  await page.goto(bridge.url);
  await createVault(page);
  await addServer(page, { name: 'Pol', host: 'sshd-password', password: PASS });

  await page.getByRole('button', { name: 'Settings' }).click();
  await expect(page.getByTestId('settings-dialog')).toBeVisible();
  await page.getByTestId('set-parallel').fill('3');
  await page.getByTestId('set-dragout').fill('50');
  await page.getByTestId('set-editlimit').fill('2');
  await page.getByTestId('set-autolock').fill('15');
  await page.getByTestId('set-mtime').uncheck();
  await page.screenshot({ path: 'screenshots/54-settings.png' });
  await page.getByTestId('settings-save').click();
  await expect(page.getByTestId('settings-dialog')).toHaveCount(0);

  // the values are in settings.json (backend), not only in the browser
  const s = await (await fetch(`${bridge.url}/rpc`, { method: 'POST', body: JSON.stringify({ method: 'get_settings', params: {} }) })).json();
  expect(s.result).toMatchObject({ parallel_transfers: 3, drag_out_limit_mb: 50, editor_limit_mb: 2, auto_lock_minutes: 15, keep_mtime: false });

  // about
  await page.getByRole('button', { name: 'Settings' }).click();
  await page.getByTestId('open-about').click();
  await expect(page.getByTestId('about-author')).toHaveText('dest1256');
  await expect(page.getByTestId('about-version')).toHaveText(/^\d+\.\d+\.\d+/);
  await page.getByTestId('open-licenses').click();
  const lic = page.getByTestId('licenses-text');
  await expect(lic).toContainText('serde');
  await expect(lic).toContainText('@codemirror/state');
  await expect(lic).toContainText('MIT License');
  await page.getByTestId('licenses-close').click();
  await page.getByTestId('about-close').click();
  await page.getByTestId('settings-cancel').click();

  // renaming a tab by double click
  await connectFirst(page);
  await acceptHostKey(page);
  await expect(page.getByTestId('tab-s1')).toBeVisible();
  await page.getByTestId('tab-s1').dblclick();
  await page.getByTestId('prompt-input').fill('My server');
  await page.getByTestId('prompt-ok').click();
  await expect(page.getByTestId('tab-s1')).toContainText('My server');
});
