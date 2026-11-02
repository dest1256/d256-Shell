import { test, expect, Page } from '@playwright/test';
import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { Bridge, startBridge } from '../fixtures';
import { addServer, createVault, PASS, row } from '../helpers';

let bridge: Bridge;
test.afterEach(async () => bridge?.stop());

type XTerm = import('@xterm/xterm').Terminal;
const buffer = (page: Page) =>
  page.evaluate(() => {
    const t = (document.querySelector('[data-testid="local-terminal"]') as unknown as { __term: XTerm }).__term;
    const b = t.buffer.active;
    const out: string[] = [];
    for (let i = 0; i < b.length; i++) out.push(b.getLine(i)?.translateToString(true) ?? '');
    return out.join('\n');
  });
const waitFor = (page: Page, text: string) => expect.poll(() => buffer(page), { timeout: 20_000 }).toContain(text);
const rpc = (page: Page, method: string, params: object = {}) =>
  page.evaluate(async ([m, p]) => (await (await fetch('/rpc', { method: 'POST', body: JSON.stringify({ method: m, params: p }) })).json()).result, [method, params] as [string, object]);

async function start(page: Page) {
  bridge = await startBridge();
  await page.goto(bridge.url);
  await createVault(page);
}

test('terminals: local shells are listed and open in a tab next to the SSH sessions', async ({ page }) => {
  await start(page);
  await page.getByTestId('rail-terminals').click();
  const items = page.getByTestId('profile-item');
  await expect(items.first()).toBeVisible();
  // the home page now shows the shells too, and only shells
  await expect(page.getByTestId('home-terminals')).toBeVisible();
  await expect(page.getByTestId('server-item')).toHaveCount(0);
  await items.first().click();
  await expect(page.getByTestId('tab-local1')).toBeVisible();
  await page.getByTestId('local-terminal').click();
  await page.keyboard.type('echo local-$((20+22))');
  await page.keyboard.press('Enter');
  await waitFor(page, 'local-42');
  await page.screenshot({ path: 'screenshots/71-local-terminal.png' });
  // closing a local shell needs no "connection will be closed" question
  await page.getByTestId('tab-close-local1').click();
  await expect(page.getByTestId('tab-local1')).toHaveCount(0);
  await expect(page.getByTestId('confirm')).toHaveCount(0);

  // an own profile from Settings-like dialog
  await page.getByTestId('profiles-edit').click();
  await page.getByTestId('prof-add').click();
  await page.getByTestId('prof-name-0').fill('My sh');
  await page.getByTestId('prof-prog-0').fill('/bin/sh');
  await page.getByTestId('prof-save').click();
  await expect(items.filter({ hasText: 'My sh' })).toBeVisible();
  await items.filter({ hasText: 'My sh' }).click();
  await page.getByTestId('local-terminal').click();
  await page.keyboard.type('echo mine-$((1+1))');
  await page.keyboard.press('Enter');
  await waitFor(page, 'mine-2');
});

test('scripts: write one in the editor and run it; link a file (never copied), grey when missing', async ({ page }) => {
  await start(page);
  await page.getByTestId('rail-scripts').click();
  await expect(page.getByTestId('script-drop')).toBeVisible();
  await page.getByTestId('sc-new').click();
  await page.getByTestId('sc-new-name').fill('hello');
  await page.getByTestId('sc-new-ext').selectOption('.sh');
  await page.getByTestId('sc-new-ok').click();
  await expect(page.getByTestId('script-editor')).toBeVisible();
  await page.locator('[data-testid="script-editor"] .cm-content').click();
  await page.keyboard.type('echo from-script-$((1+2))');
  await page.keyboard.press('Control+s');
  await expect(page.getByTestId('sc-status')).toHaveText('Saved');
  expect(readFileSync(join(bridge.dataDir, 'scripts', 'hello.sh'), 'utf8')).toBe('echo from-script-$((1+2))');
  await page.getByTestId('sc-run').click();
  await waitFor(page, 'from-script-3');
  await page.screenshot({ path: 'screenshots/72-script-run.png' });
  await page.getByTestId('rail-scripts').click(); // back to the list (the side panel is hidden while a tab is busy)

  // a link: the file stays where it is
  const dir = join(tmpdir(), `d256shell-e2e-links-${Date.now()}`);
  mkdirSync(dir, { recursive: true });
  const file = join(dir, 'tool.sh');
  writeFileSync(file, 'echo linked\n');
  await page.getByTestId('sc-link').click();
  await page.getByTestId('prompt-input').fill(file);
  await page.getByTestId('prompt-ok').click();
  const item = page.getByTestId('script-item').filter({ hasText: 'tool.sh' });
  await expect(item).toBeVisible();
  await expect(item).toContainText('🔗');
  expect(existsSync(join(bridge.dataDir, 'scripts', 'tool.sh'))).toBe(false);
  await page.screenshot({ path: 'screenshots/73-scripts-list.png' });

  // missing: grey, with a reason; the entry stays
  renameSync(file, file + '.gone');
  await page.getByTestId('rail-sessions').click();
  await page.getByTestId('rail-scripts').click();
  await expect(page.getByTestId('script-item').filter({ hasText: 'tool.sh' })).toHaveAttribute('data-missing', '1');
  await expect(page.getByTestId('script-item').filter({ hasText: 'tool.sh' })).toContainText('file not found');

  // removing a link removes only the entry
  await page.getByTestId('script-item').filter({ hasText: 'tool.sh' }).hover();
  await page.getByTestId('script-item').filter({ hasText: 'tool.sh' }).getByTestId('script-delete').click();
  await expect(page.getByTestId('script-item').filter({ hasText: 'tool.sh' })).toHaveCount(0);
  expect(existsSync(file + '.gone')).toBe(true);
  // deleting an own script asks first and removes the file
  const own = page.getByTestId('script-item').filter({ hasText: 'hello.sh' });
  await own.hover();
  await own.getByTestId('script-delete').click();
  await page.getByTestId('confirm-ok').click();
  await expect(own).toHaveCount(0);
  expect(existsSync(join(bridge.dataDir, 'scripts', 'hello.sh'))).toBe(false);
});

test('notes: any title, saved by itself, encrypted in the vault', async ({ page }) => {
  await start(page);
  await page.getByTestId('rail-notes').click();
  await expect(page.getByTestId('note-list')).toContainText('No notes yet');
  await page.getByTestId('note-new').click();
  await page.getByTestId('note-title').fill('VPS passwords');
  await page.locator('[data-testid="note-editor"] .cm-content').click();
  await page.keyboard.type('root: very-secret-123');
  await expect(page.getByTestId('note-status')).toHaveText('Saved', { timeout: 10_000 });
  await page.screenshot({ path: 'screenshots/74-note.png' });
  await page.getByTestId('rail-notes').click();
  await expect(page.getByTestId('note-item').filter({ hasText: 'VPS passwords' })).toBeVisible();
  // encrypted at rest
  expect(readFileSync(join(bridge.dataDir, 'vault.bin')).toString('latin1')).not.toContain('very-secret-123');

  // lock and unlock: the note is still there
  await page.getByTestId('lock').click();
  await page.getByTestId('master').fill('e2e-master-pass');
  await page.getByTestId('unlock').click();
  await page.getByTestId('rail-notes').click();
  await page.getByTestId('note-item').filter({ hasText: 'VPS passwords' }).click();
  await expect(page.locator('[data-testid="note-editor"] .cm-content')).toContainText('very-secret-123');

  // delete with confirmation
  await page.getByTestId('note-delete').click();
  await page.getByTestId('confirm-ok').click();
  await expect(page.getByTestId('note-item')).toHaveCount(0);
  expect(((await rpc(page, 'note_list')) as { notes: unknown[] }).notes).toHaveLength(0);
});

test('icons: a built-in glyph or your own image for a session; remembered', async ({ page }) => {
  await start(page);
  await addServer(page, { name: 'Iconic', host: 'sshd-password', password: PASS });
  const row = page.getByTestId('server-item').first();
  await row.hover();
  await row.getByTestId('server-icon').click();
  await page.getByTestId('icon-bolt').click();
  const icons = async () => Object.values(((await rpc(page, 'get_settings')) as { icons: Record<string, string> }).icons);
  await expect.poll(icons).toEqual(['bolt']);

  // an own image is copied into data/icons and shown as <img>
  const png = join(tmpdir(), `d256shell-e2e-icon-${Date.now()}.png`);
  writeFileSync(png, Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==', 'base64'));
  await row.hover();
  await row.getByTestId('server-icon').click();
  await page.getByTestId('icon-file').click();
  await page.getByTestId('prompt-input').fill(png);
  await page.getByTestId('prompt-ok').click();
  await expect(page.locator('[data-testid="server-item"] img.custom-ico').first()).toBeVisible();
  await expect.poll(async () => (await icons())[0]).toMatch(/.png$/);
  const file = (await icons())[0];
  expect(file).toMatch(/\.png$/);
  expect(existsSync(join(bridge.dataDir, 'icons', file))).toBe(true);
  await page.screenshot({ path: 'screenshots/75-icons.png' });
  // back to the default
  await row.hover();
  await row.getByTestId('server-icon').click();
  await page.getByTestId('icon-reset').click();
  await expect.poll(async () => Object.keys(((await rpc(page, 'get_settings')) as { icons: object }).icons).length).toBe(0);
});

test('tabs on the left and the shell that opens at start', async ({ page }) => {
  await start(page);
  await page.getByRole('button', { name: 'Settings' }).click();
  await page.getByTestId('set-tabs').selectOption('left');
  const startup = page.getByTestId('set-startup');
  const first = (await startup.locator('option').evaluateAll((os) => os.map((o) => (o as HTMLOptionElement).value))).find((v) => v.startsWith('auto:'));
  expect(first).toBeTruthy();
  await startup.selectOption(first!);
  await page.getByTestId('settings-save').click();
  // tabs moved to the left column
  await expect(page.getByTestId('open-tabs')).toBeVisible();
  await expect(page.getByTestId('vtab-home')).toBeVisible();
  await expect(page.locator('.titlebar .tabs')).toBeHidden();
  // the startup shell opens after a restart of the UI
  await page.reload();
  await expect(page.getByTestId('tab-local1')).toHaveCount(1);
  await expect(page.getByTestId('vtab-local1')).toBeVisible();
  await page.getByTestId('vtab-home').click();
  await expect(page.getByTestId('home')).toBeVisible();
  await page.screenshot({ path: 'screenshots/76-left-tabs.png' });
  await page.getByTestId('vtab-close-local1').click();
  await expect(page.getByTestId('vtab-local1')).toHaveCount(0);
});

test('local explorer: folders of this computer on the left of a local terminal, like the SFTP one', async ({ page }) => {
  await start(page);
  const dir = join(tmpdir(), `d256shell-e2e-lfs-${Date.now()}`);
  mkdirSync(join(dir, 'sub'), { recursive: true });
  writeFileSync(join(dir, 'a.txt'), 'first\n');
  writeFileSync(join(dir, '.hidden'), 'h');

  await page.getByTestId('rail-terminals').click();
  await page.getByTestId('profile-item').first().click();
  const ex = page.getByTestId('local-ex');
  await expect(ex).toBeVisible();
  // explorer on the left, terminal on the right
  const exBox = (await ex.boundingBox())!;
  const termBox = (await page.getByTestId('local-terminal').boundingBox())!;
  expect(exBox.x + exBox.width).toBeLessThanOrEqual(termBox.x + 1);
  // the SFTP-only actions are not offered
  await expect(ex.getByTestId('upload')).toHaveCount(0);
  await expect(ex.getByTestId('download')).toHaveCount(0);

  await ex.getByTestId('path-input').fill(dir);
  await ex.getByTestId('path-input').press('Enter');
  await expect(ex.getByTestId('path-input')).toHaveValue(dir);
  await expect(row(page, 'a.txt')).toBeVisible();
  await expect(row(page, 'sub')).toBeVisible();
  await expect(row(page, '.hidden')).toBeVisible(); // hidden files are shown (the toggle is on by default)

  // create, rename, copy, paste
  await ex.getByTestId('new-folder').click();
  await page.getByTestId('prompt-input').fill('made');
  await page.getByTestId('prompt-ok').click();
  await expect(row(page, 'made')).toBeVisible();
  expect(existsSync(join(dir, 'made'))).toBe(true);
  await row(page, 'a.txt').click();
  await page.keyboard.press('F2');
  await page.getByTestId('prompt-input').fill('b.txt');
  await page.getByTestId('prompt-ok').click();
  await expect(row(page, 'b.txt')).toBeVisible();
  expect(existsSync(join(dir, 'a.txt'))).toBe(false);
  await page.keyboard.press('Control+c');
  await page.keyboard.press('Control+v');
  await expect(row(page, 'b (copy).txt')).toBeVisible();
  expect(readFileSync(join(dir, 'b (copy).txt'), 'utf8')).toBe('first\n');

  // open a text file in the built-in editor, change it, save
  await row(page, 'b.txt').dblclick();
  await expect(page.getByTestId('file-tab')).toBeVisible();
  await page.locator('[data-testid="script-editor"] .cm-content').click();
  await page.keyboard.press('Control+End');
  await page.keyboard.type('second');
  await page.keyboard.press('Control+s');
  await expect(page.getByTestId('sc-status')).toHaveText('Saved');
  expect(readFileSync(join(dir, 'b.txt'), 'utf8')).toBe('first\nsecond');
  await expect(page.getByTestId('sc-run')).toHaveCount(0);
  await page.screenshot({ path: 'screenshots/77-local-explorer-editor.png' });

  // back to the terminal tab: delete with confirmation, the folder goes up with ".."
  await page.getByTestId('tab-local1').click();
  await row(page, 'made').click();
  await page.keyboard.press('Delete');
  await page.getByTestId('confirm-ok').click();
  await expect(row(page, 'made')).toHaveCount(0);
  expect(existsSync(join(dir, 'made'))).toBe(false);
  await row(page, 'sub').dblclick();
  await expect(ex.getByTestId('path-input')).toHaveValue(join(dir, 'sub'));
  await ex.getByTestId('up').click();
  await expect(ex.getByTestId('path-input')).toHaveValue(dir);
  await page.screenshot({ path: 'screenshots/78-local-explorer.png' });
});
