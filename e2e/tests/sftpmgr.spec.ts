import { test, expect, Page } from '@playwright/test';
import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync, utimesSync } from 'node:fs';
import { homedir, tmpdir } from 'node:os';
import { join } from 'node:path';
import { Bridge, startBridge } from '../fixtures';
import { acceptHostKey, addServer, connectFirst, createVault, PASS, rpc } from '../helpers';

let bridge: Bridge;
test.afterEach(async () => bridge?.stop());

/** A fresh folder on this computer (the test and the program run on the same machine). */
function scratch(name: string): string {
  const d = join(tmpdir(), `d256shell-sm-${name}-${Date.now()}`);
  mkdirSync(d, { recursive: true });
  return d;
}

async function openManager(page: Page) {
  bridge = await startBridge();
  await page.goto(bridge.url);
  await createVault(page);
  await addServer(page, { name: 'Mgr', host: 'sshd-password', password: PASS });
  await connectFirst(page);
  await acceptHostKey(page);
  await expect(page.getByTestId('tab-s1')).toBeVisible();
  await page.getByTestId('sftp-open').click();
  await expect(page.getByTestId('tab-sftp1')).toBeVisible({ timeout: 20_000 });
  await expect(page.getByTestId('sftp-manager')).toBeVisible();
}

const pane = (page: Page, side: 'local' | 'remote') => page.getByTestId(`pane-${side}`);
const row = (page: Page, side: 'local' | 'remote', name: string) => pane(page, side).locator(`[data-testid="explorer-row"][data-name="${name}"]`);
async function goLocal(page: Page, dir: string) {
  const input = pane(page, 'local').getByTestId('path-input');
  await input.fill(dir);
  await input.press('Enter');
  await expect(input).toHaveValue(dir);
}
const jobs = (page: Page, state: string) => page.locator(`[data-testid="sm-job"][data-state="${state}"]`);

test('the SFTP manager: two panes with trees, the status log, the function keys and a selection summary', async ({ page }) => {
  await openManager(page);
  // the server is on the right: the home folder, the owner of every entry, the tree of folders
  await expect(pane(page, 'remote').getByTestId('path-input')).toHaveValue('/home/tester');
  await expect(row(page, 'remote', 'zone')).toBeVisible();
  await expect(row(page, 'remote', 'zone').locator('.c-owner')).toHaveText('root');
  await expect(pane(page, 'remote').getByTestId('tree-remote').locator('.tn', { hasText: 'home' }).first()).toBeVisible();
  await expect(pane(page, 'remote').getByTestId('tree-remote').locator('.tn.sel')).toContainText('tester'); // the tree follows the list
  // this computer on the left: no owner column (no names there), a tree starting at the root of the computer
  await expect(pane(page, 'local').getByTestId('tree-local')).toBeVisible();
  await expect(pane(page, 'local').locator('.c-owner')).toHaveCount(0);
  // the status log is impersonal: what happens, not who does it
  const log = page.getByTestId('sm-log');
  await expect(log).toContainText('SFTP session open');
  await expect(log).toContainText('The listing of “/home/tester” is complete');
  await expect(log).not.toContainText(/\bI\b/);
  // the function keys
  const keys = await page.getByTestId('sm-fkeys').locator('button').allTextContents();
  expect(keys).toEqual(['F3View', 'F4Edit', 'F5Copy', 'F6Move', 'F7New folder', 'F8Delete', 'F2Rename', 'F9Terminal', 'Alt+XClose tab']);
  // the selection summary under the list
  await row(page, 'remote', 'zone').click();
  await expect(pane(page, 'remote').getByTestId('explorer-status')).toContainText('folders: 1 of ');
  await page.screenshot({ path: 'screenshots/89-sftp-manager.png' });
  // clicking a folder of the tree opens it in the list
  await pane(page, 'remote').getByTestId('tree-remote').locator('.tn', { hasText: 'zone' }).first().click();
  await expect(pane(page, 'remote').getByTestId('path-input')).toHaveValue('/home/tester/zone');
  // Tab switches the active pane
  await expect(pane(page, 'remote')).toHaveClass(/active/);
  await row(page, 'remote', '..').click();
  await page.keyboard.press('Tab');
  await expect(pane(page, 'local')).toHaveClass(/active/);
});

test('F5 copies to the other pane both ways, F6 moves; the queue shows the jobs; a local delete goes to the trash', async ({ page }) => {
  await openManager(page);
  const up = scratch('up');
  const down = scratch('down');
  writeFileSync(join(up, 'up-file.txt'), 'to the server');
  writeFileSync(join(up, 'move-me.txt'), 'moved');
  writeFileSync(join(up, 'trash-me.txt'), 'bye');
  await goLocal(page, up);
  await expect(row(page, 'local', 'up-file.txt')).toBeVisible();

  // local -> server (F5)
  await row(page, 'local', 'up-file.txt').click();
  await page.keyboard.press('F5');
  await page.getByTestId('sm-qtab-done').click();
  await expect(jobs(page, 'done')).toHaveCount(1, { timeout: 20_000 });
  await expect(row(page, 'remote', 'up-file.txt')).toBeVisible({ timeout: 20_000 }); // the pane that received the file shows it
  await expect(page.getByTestId('sm-log')).toContainText('Transfer finished: up-file.txt');
  await page.screenshot({ path: 'screenshots/90-sftp-queue.png' });

  // server -> this computer (F5 from the right pane), into the folder of the left pane
  await goLocal(page, down);
  await row(page, 'remote', 'up-file.txt').click();
  await page.keyboard.press('F5');
  await expect.poll(() => existsSync(join(down, 'up-file.txt')), { timeout: 20_000 }).toBe(true);
  expect(readFileSync(join(down, 'up-file.txt'), 'utf8')).toBe('to the server');

  // F6: copied, then the original is gone (a local original goes to the trash)
  await goLocal(page, up);
  await row(page, 'local', 'move-me.txt').click();
  await page.keyboard.press('F6');
  await expect(page.getByRole('dialog').filter({ hasText: 'moved to the trash here' })).toBeVisible();
  await page.getByTestId('confirm-ok').click();
  await expect(row(page, 'remote', 'move-me.txt')).toBeVisible({ timeout: 20_000 });
  await expect(row(page, 'local', 'move-me.txt')).toHaveCount(0, { timeout: 20_000 });
  expect(existsSync(join(up, 'move-me.txt'))).toBe(false);

  // F6 from the server: the file arrives here, then it is deleted on the server
  await goLocal(page, down);
  await row(page, 'remote', 'move-me.txt').click();
  await page.keyboard.press('F6');
  await page.getByTestId('confirm-ok').click();
  await expect.poll(() => existsSync(join(down, 'move-me.txt')), { timeout: 20_000 }).toBe(true);
  await expect(row(page, 'remote', 'move-me.txt')).toHaveCount(0, { timeout: 20_000 });

  // F8 on the left pane: to the trash, not for good
  await goLocal(page, up);
  await row(page, 'local', 'trash-me.txt').click();
  await page.keyboard.press('F8');
  await expect(page.getByRole('dialog').filter({ hasText: 'Moved to the trash' })).toBeVisible();
  await page.getByTestId('confirm-ok').click();
  await expect(row(page, 'local', 'trash-me.txt')).toHaveCount(0, { timeout: 10_000 });
  const trashed = join(process.env.XDG_DATA_HOME || join(homedir(), '.local/share'), 'Trash/files/trash-me.txt');
  expect(existsSync(trashed)).toBe(true);
  // the server side of F8 deletes for good (after a question)
  await row(page, 'remote', 'up-file.txt').click();
  await page.keyboard.press('F8');
  await expect(page.getByRole('dialog').filter({ hasText: 'Permanently deleted on the server' })).toBeVisible();
  await page.getByTestId('confirm-ok').click();
  await expect(row(page, 'remote', 'up-file.txt')).toHaveCount(0, { timeout: 10_000 });
});

test('F7 makes a folder, F2 renames, Insert selects several, a name typed jumps to it, F4 opens the editor', async ({ page }) => {
  await openManager(page);
  const dir = scratch('keys');
  writeFileSync(join(dir, 'alpha.txt'), 'a');
  writeFileSync(join(dir, 'beta.txt'), 'b');
  writeFileSync(join(dir, 'gamma.txt'), 'g');
  await goLocal(page, dir);
  await row(page, 'local', 'alpha.txt').click();
  // type-ahead: "g" jumps to gamma
  await page.keyboard.type('g');
  await expect(pane(page, 'local').locator('.vrow.cursor')).toHaveAttribute('data-name', 'gamma.txt');
  // Insert marks and goes down
  await row(page, 'local', 'alpha.txt').click(); // selected
  await page.keyboard.press('Insert'); // alpha off, on to beta
  await page.keyboard.press('Insert'); // beta on
  await page.keyboard.press('Insert'); // gamma on
  await expect(pane(page, 'local').getByTestId('explorer-status')).toContainText('files: 2 of 3');
  // F7: a new folder
  await page.keyboard.press('F7');
  await page.getByTestId('prompt-input').fill('made-here');
  await page.getByTestId('prompt-ok').click();
  await expect(row(page, 'local', 'made-here')).toBeVisible();
  // F2: rename
  await row(page, 'local', 'beta.txt').click();
  await page.keyboard.press('F2');
  await page.getByTestId('prompt-input').fill('beta-renamed.txt');
  await page.getByTestId('prompt-ok').click();
  await expect(row(page, 'local', 'beta-renamed.txt')).toBeVisible();
  // F4: the editor opens next to the files, as a pane of the tab
  await row(page, 'local', 'gamma.txt').click();
  await page.keyboard.press('F4');
  await expect(page.getByTestId('script-editor')).toBeVisible();
  await expect(page.getByTestId('pane-files')).toBeVisible();
  await page.getByTestId('pane-files').click();
  await expect(pane(page, 'local')).toBeVisible();
  // the same on the server: a file opens in the remote editor
  await rpc(bridge.url, 'noop').catch(() => undefined);
  await row(page, 'remote', '.bashrc').click();
  await page.keyboard.press('Tab'); // the active pane is the right one: Tab goes to the left; click the row again to be sure
  await row(page, 'remote', '.bashrc').click();
  await page.keyboard.press('F3');
  await expect(page.getByTestId('editor')).toBeVisible({ timeout: 20_000 });
});

test('compare the folders, the quick filter, hide the log and the trees; the choice is remembered', async ({ page }) => {
  await openManager(page);
  const dir = scratch('cmp');
  writeFileSync(join(dir, 'only-here.txt'), 'x');
  writeFileSync(join(dir, '.bashrc'), 'a different size than the one on the server'); // a name that exists on the server
  const old = new Date(Date.now() - 86_400_000 * 30);
  utimesSync(join(dir, '.bashrc'), old, old);
  await goLocal(page, dir);
  // "/home/tester" has .bashrc, zone, .config ...: left has only-here.txt and its own .bashrc
  await page.getByTestId('sm-compare').click();
  await expect(row(page, 'local', 'only-here.txt')).toHaveClass(/mark-only/);
  await expect(row(page, 'local', '.bashrc')).toHaveClass(/mark-diff/);
  await expect(row(page, 'remote', '.bashrc')).toHaveClass(/mark-diff/);
  await expect(row(page, 'remote', 'zone')).toHaveClass(/mark-only/);
  await expect(page.getByTestId('sm-log')).toContainText('The folders are compared');
  await page.screenshot({ path: 'screenshots/91-sftp-compare.png' });
  await page.getByTestId('sm-compare').click();
  await expect(row(page, 'remote', 'zone')).not.toHaveClass(/mark-only/);

  // the quick filter: both panes have a field; the list keeps what matches
  await page.getByTestId('sm-filter').click();
  await pane(page, 'remote').getByTestId('filter-remote').fill('zon');
  await expect(row(page, 'remote', 'zone')).toBeVisible();
  await expect(row(page, 'remote', '.bashrc')).toHaveCount(0);
  await pane(page, 'remote').getByTestId('filter-remote').press('Escape');
  await expect(row(page, 'remote', '.bashrc')).toBeVisible();
  await page.getByTestId('sm-filter').click(); // off: the fields go away

  // the log and the trees can be hidden, and Settings has the same switches
  await page.getByTestId('sm-toggle-log').click();
  await page.getByTestId('sm-toggle-trees').click();
  await expect(page.getByTestId('sm-log')).toBeHidden();
  await expect(page.getByTestId('tree-local')).toBeHidden();
  await expect(page.getByTestId('tree-remote')).toBeHidden();
  await expect(row(page, 'remote', 'zone')).toBeVisible(); // the lists take the room
  await page.screenshot({ path: 'screenshots/92-sftp-hidden.png' });
  await expect.poll(async () => ((await rpc(bridge.url, 'get_settings')) as { sftp_log: boolean; sftp_trees: boolean }).sftp_log).toBe(false);
  await page.getByRole('button', { name: 'Settings' }).click();
  await page.getByTestId('settings-tab-files').click();
  await expect(page.getByTestId('set-sftp-log')).not.toBeChecked();
  await expect(page.getByTestId('set-sftp-trees')).not.toBeChecked();
  await expect(page.getByTestId('set-sftp-keys')).toBeChecked();
  await page.getByTestId('set-sftp-keys').uncheck();
  await page.getByTestId('set-sftp-log').check();
  await page.getByTestId('settings-save').click();
  await expect(page.getByTestId('sm-fkeys')).toBeHidden();
  await expect(page.getByTestId('sm-log')).toBeVisible();
  await expect(page.getByTestId('tree-local')).toBeHidden();
});

test('Settings has tabs; closing the manager tab keeps the SSH tab alive', async ({ page }) => {
  await openManager(page);
  await page.getByRole('button', { name: 'Settings' }).click();
  await expect(page.getByTestId('settings-dialog')).toBeVisible();
  const tabs = await page.getByTestId('settings-tabs').locator('button').allTextContents();
  expect(tabs).toEqual(['General', 'Terminal', 'Files and SFTP', 'Monitor', 'Security', 'About']);
  await expect(page.getByTestId('settings-panel-general')).toBeVisible();
  await expect(page.getByTestId('settings-panel-terminal')).toBeHidden();
  for (const id of ['terminal', 'files', 'monitor', 'security', 'about', 'general']) {
    await page.getByTestId(`settings-tab-${id}`).click();
    await expect(page.getByTestId(`settings-panel-${id}`)).toBeVisible();
  }
  await page.getByTestId('settings-tab-files').click();
  await page.screenshot({ path: 'screenshots/93-settings-tabs.png' });
  await page.getByTestId('settings-cancel').click();
  // the manager and the SSH tab are two tabs of one server; closing one does not touch the other
  await page.getByTestId('tab-close-sftp1').click();
  await page.getByTestId('confirm-ok').click();
  await expect(page.getByTestId('tab-sftp1')).toHaveCount(0);
  await expect(page.getByTestId('tab-s1')).toBeVisible();
  await page.getByTestId('tab-s1').click();
  await expect(page.getByTestId('path-input')).toHaveValue('/home/tester');
  await page.getByTestId('tab-s1').click({ button: 'right' });
  await page.getByTestId('tabmenu-sftp').click();
  await expect(page.getByTestId('tab-sftp2')).toBeVisible({ timeout: 20_000 });
});

test('the server password stays encrypted: nowhere in the data folder in plain text, also after the manager worked', async ({ page }) => {
  await openManager(page);
  const dir = scratch('pw');
  writeFileSync(join(dir, 'p.txt'), 'x');
  await goLocal(page, dir);
  await row(page, 'local', 'p.txt').click();
  await page.keyboard.press('F5');
  await expect(row(page, 'remote', 'p.txt')).toBeVisible({ timeout: 20_000 });
  const files: string[] = [];
  const walk = (d: string) => {
    for (const e of readdirSync(d, { withFileTypes: true })) {
      const p = join(d, e.name);
      if (e.isDirectory()) {
        if (e.name !== 'webview') walk(p);
      } else files.push(p);
    }
  };
  walk(bridge.dataDir);
  expect(files.some((f) => f.endsWith('vault.bin'))).toBe(true); // the password lives here, encrypted
  for (const f of files) expect(readFileSync(f).includes(Buffer.from(PASS)), `${f} must not contain the password`).toBe(false);
});
