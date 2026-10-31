import { test, expect } from '@playwright/test';
import { readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { Bridge, startBridge } from '../fixtures';
import { acceptHostKey, addServer, connectFirst, createVault, PASS, row, totp } from '../helpers';

let bridge: Bridge;
test.afterEach(async () => bridge?.stop());

async function start(page: import('@playwright/test').Page) {
  bridge = await startBridge();
  await page.goto(bridge.url);
  await createVault(page);
}

test('connection: host key, explorer, navigation, hidden files, symlinks', async ({ page }) => {
  await start(page);
  await addServer(page, { name: 'Test-1', host: 'sshd-password', password: PASS });
  await connectFirst(page);
  await acceptHostKey(page);
  await page.screenshot({ path: 'screenshots/20-connecting.png' });

  await expect(page.getByTestId('tab-s1')).toBeVisible();
  await expect(page.getByTestId('path-input')).toHaveValue('/home/tester');
  await expect(row(page, 'zone')).toBeVisible();
  await expect(row(page, '.bashrc')).toBeVisible(); // hidden files are visible by default
  await page.getByTestId('hidden-toggle').click();
  await expect(row(page, '.bashrc')).toHaveCount(0);
  await page.getByTestId('hidden-toggle').click();
  await expect(row(page, '.bashrc')).toBeVisible();

  // into the folder zone
  await row(page, 'zone').dblclick();
  await expect(page.getByTestId('path-input')).toHaveValue('/home/tester/zone');
  for (const n of ['юникод-файл.txt', 'file with spaces.txt', '-dash.txt', "it's $weird *name.txt", 'dir with spaces']) {
    await expect(row(page, n), n).toBeVisible();
  }
  // folders come first
  const kinds = await page.locator('[data-testid="explorer-row"]').evaluateAll((els) => els.map((e) => (e as HTMLElement).dataset.kind));
  const firstFile = kinds.findIndex((k) => k === 'file');
  expect(kinds.slice(0, firstFile).every((k) => k === 'dir' || k === 'symlink')).toBe(true);

  // a non-UTF-8 name is marked and does not open
  const lossy = page.locator('[data-testid="explorer-row"].lossy');
  await expect(lossy).toHaveCount(1);
  await lossy.dblclick();
  await expect(page.getByRole('alert')).toContainText('UTF-8');

  // sorting by size changes the indicator
  await page.getByTestId('sort-size').click();
  await expect(page.getByTestId('sort-size')).toContainText('▲');
  await page.getByTestId('sort-size').click();
  await expect(page.getByTestId('sort-size')).toContainText('▼');
  await page.screenshot({ path: 'screenshots/21-explorer.png' });

  // symlink to a folder: it opens, the path is canonicalised
  await row(page, 'link_to_protected').dblclick();
  await expect(page.getByTestId('path-input')).toHaveValue('/protected');
  await expect(row(page, 'a.txt')).toBeVisible();

  // up / Backspace / home / address bar
  await page.getByTestId('up').click();
  await expect(page.getByTestId('path-input')).toHaveValue('/');
  await page.getByTestId('ex-home').click();
  await expect(page.getByTestId('path-input')).toHaveValue('/home/tester');
  await page.getByTestId('path-input').fill('/protected');
  await page.getByTestId('path-input').press('Enter');
  await expect(row(page, 'b.txt')).toBeVisible();
  await page.getByTestId('explorer-list').focus();
  await page.keyboard.press('Backspace');
  await expect(page.getByTestId('path-input')).toHaveValue('/');

  // no access: a clear error, the current folder stays
  await page.getByTestId('path-input').fill('/root');
  await page.getByTestId('path-input').press('Enter');
  await expect(page.getByRole('alert').filter({ hasText: 'No access rights' })).toBeVisible();
  await expect(page.getByTestId('path-input')).toHaveValue('/');

  // second connection to the same server: the key is already known, no dialog
  await page.getByTestId('tab-home').click();
  await page.getByTestId('server-item').first().dblclick();
  await expect(page.getByTestId('tab-s2')).toBeVisible();
  await expect(page.getByTestId('hostkey-dialog')).toHaveCount(0);

  // closing a tab with confirmation
  await page.getByTestId('tab-close-s2').click();
  await page.getByTestId('confirm-cancel').click();
  await expect(page.getByTestId('tab-s2')).toBeVisible();
  await page.getByTestId('tab-close-s2').click();
  await page.getByTestId('confirm-ok').click();
  await expect(page.getByTestId('tab-s2')).toHaveCount(0);
  await expect(page.getByTestId('tab-s1')).toBeVisible();
});

test('10,000 files: the list is virtualised', async ({ page }) => {
  await start(page);
  await addServer(page, { name: 'Many', host: 'sshd-password', password: PASS });
  await connectFirst(page);
  await acceptHostKey(page);
  await expect(page.getByTestId('tab-s1')).toBeVisible();
  await page.getByTestId('path-input').fill('/home/tester/zone/many');
  await page.getByTestId('path-input').press('Enter');
  await expect(page.getByTestId('explorer-status')).toContainText('Objects: 10000');
  expect(await page.locator('[data-testid="explorer-row"]').count()).toBeLessThan(100);
  await page.getByTestId('explorer-list').evaluate((el) => (el.scrollTop = el.scrollHeight));
  await expect(row(page, 'f10000')).toBeVisible(); // numeric sort: the last one
  expect(await page.locator('[data-testid="explorer-row"]').count()).toBeLessThan(100);
  // Ctrl+A selects everything
  await page.getByTestId('explorer-list').focus();
  await page.keyboard.press('Control+a');
  await expect(page.getByTestId('explorer-status')).toContainText('selected: 10000');
});

test('wrong saved password: one attempt, manual entry, saving', async ({ page }) => {
  await start(page);
  await addServer(page, { name: 'WrongPw', host: 'sshd-password', password: 'old-wrong-password' });
  await connectFirst(page);
  await acceptHostKey(page);
  await expect(page.getByTestId('wrong-password')).toBeVisible();
  await expect(page.getByTestId('retry-save')).toBeChecked();
  await page.screenshot({ path: 'screenshots/22-wrong-password.png' });
  await page.getByTestId('retry-password').fill(PASS);
  await page.getByTestId('retry-ok').click();
  await expect(page.getByTestId('tab-s1')).toBeVisible();
  await expect(page.getByTestId('hostkey-dialog')).toHaveCount(0);

  // the new password is saved in the vault
  await page.getByTestId('tab-home').click();
  await page.getByTestId('server-item').first().hover();
  await page.getByTestId('edit').click();
  await page.getByTestId('eye').click();
  await expect(page.getByTestId('f-password')).toHaveValue(PASS);
});

test('the two-factor code is requested from the user', async ({ page }) => {
  await start(page);
  await addServer(page, { name: '2FA', host: 'sshd-2fa', password: PASS });
  await connectFirst(page);
  await acceptHostKey(page);
  await expect(page.getByTestId('prompt-dialog')).toBeVisible();
  await expect(page.getByTestId('prompt-text')).toContainText(/verification code/i);
  await page.screenshot({ path: 'screenshots/24-2fa.png' });
  await page.getByTestId('prompt-input').fill(totp());
  await page.getByTestId('prompt-ok').click();
  await expect(page.getByTestId('tab-s1')).toBeVisible();
  await expect(row(page, 'zone')).toBeVisible();
});

test('server key changed: refusal, warning, replacement only by an explicit action', async ({ page }) => {
  await start(page);
  // Remember the key of sshd-otherkey, then pass it off as the key of sshd-password.
  await addServer(page, { name: 'Other', host: 'sshd-otherkey', password: PASS });
  await connectFirst(page);
  await acceptHostKey(page);
  await expect(page.getByTestId('tab-s1')).toBeVisible();
  await page.getByTestId('tab-close-s1').click();
  await page.getByTestId('confirm-ok').click();

  const kh = join(bridge.dataDir, 'known_hosts');
  writeFileSync(kh, readFileSync(kh, 'utf8').replace('sshd-otherkey', 'sshd-password'));
  const before = readFileSync(kh, 'utf8');

  await addServer(page, { name: 'Victim', host: 'sshd-password', password: PASS });
  await page.getByTestId('server-item').filter({ hasText: 'Victim' }).dblclick();
  await expect(page.getByTestId('hostkey-changed')).toBeVisible();
  await expect(page.getByTestId('hostkey-changed')).toContainText('connection was NOT made');
  await page.screenshot({ path: 'screenshots/23-changed-key.png' });
  await page.getByTestId('changed-close').click();
  await expect(page.getByTestId('tab-s1')).toHaveCount(0);
  expect(readFileSync(kh, 'utf8')).toBe(before); // known_hosts untouched

  // explicit replacement with a second confirmation
  await page.getByTestId('server-item').filter({ hasText: 'Victim' }).dblclick();
  await page.getByTestId('changed-replace').click();
  await page.getByTestId('confirm-cancel').click(); // changed our mind at the second confirmation
  expect(readFileSync(kh, 'utf8')).toBe(before);

  await page.getByTestId('server-item').filter({ hasText: 'Victim' }).dblclick();
  await page.getByTestId('changed-replace').click();
  await page.getByTestId('confirm-ok').click();
  await expect(page.getByTestId('tab-s2')).toBeVisible();
  await expect(row(page, 'zone')).toBeVisible();
});

test('quick connect user@host asks for the password', async ({ page }) => {
  await start(page);
  await page.getByTestId('quick').fill('not an address');
  await page.getByTestId('quick').press('Enter');
  await expect(page.getByRole('alert')).toContainText('user@host');
  await page.getByTestId('quick').fill('tester@sshd-password');
  await page.getByTestId('quick').press('Enter');
  await acceptHostKey(page);
  await expect(page.getByTestId('prompt-dialog')).toBeVisible();
  await page.getByTestId('prompt-input').fill(PASS);
  await page.getByTestId('prompt-ok').click();
  await expect(page.getByTestId('tab-s1')).toBeVisible();
});

test('Test connection in the server form', async ({ page }) => {
  await start(page);
  await page.getByTestId('add').click();
  await page.getByTestId('f-name').fill('Check');
  await page.getByTestId('f-host').fill('sshd-password');
  await page.getByTestId('f-user').fill('tester');
  await page.getByTestId('f-password').fill(PASS);
  await page.getByTestId('form-test').click();
  await acceptHostKey(page);
  await expect(page.getByRole('alert').filter({ hasText: 'Connection successful' })).toBeVisible();
  await expect(page.getByTestId('tab-s1')).toHaveCount(0); // the session is not left open
  await expect(page.getByTestId('server-form')).toBeVisible();
});

test('cancelling the connection to an unreachable server', async ({ page }) => {
  await start(page);
  await addServer(page, { name: 'Dead', host: '10.255.255.1', password: 'x' });
  await connectFirst(page);
  await expect(page.getByTestId('connecting')).toBeVisible();
  await page.getByTestId('connect-cancel').click();
  await expect(page.getByTestId('connecting')).toHaveCount(0);
  await expect(page.getByTestId('tab-s1')).toHaveCount(0);
});
