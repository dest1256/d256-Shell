import { test, expect, Page } from '@playwright/test';
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, existsSync, readdirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { Bridge, startBridge } from '../fixtures';
import { acceptHostKey, addServer, connectFirst, createVault, PASS, removeOnServer, row, serverFile } from '../helpers';

let bridge: Bridge;
let workDir = '';
test.afterEach(async () => {
  if (workDir) await removeOnServer(bridge.url, workDir).catch(() => undefined);
  workDir = '';
  await bridge?.stop();
});

async function open(page: Page): Promise<string> {
  bridge = await startBridge();
  await page.goto(bridge.url);
  await createVault(page);
  await addServer(page, { name: 'Xfer', host: 'sshd-password', password: PASS });
  await connectFirst(page);
  await acceptHostKey(page);
  await expect(page.getByTestId('path-input')).toHaveValue('/home/tester');
  // working folder on the server
  const work = `e2e-x-${Date.now()}`;
  await page.getByTestId('new-folder').click();
  await page.getByTestId('prompt-input').fill(work);
  await page.getByTestId('prompt-ok').click();
  await row(page, work).dblclick();
  await expect(page.getByTestId('path-input')).toHaveValue(`/home/tester/${work}`);
  workDir = `/home/tester/${work}`;
  return workDir;
}

const prompt = async (page: Page, text: string) => {
  await page.getByTestId('prompt-input').fill(text);
  await page.getByTestId('prompt-ok').click();
};

test('upload and download: picker dialog (mock), transfer queue, conflicts', async ({ page }) => {
  const work = await open(page);
  const dir = mkdtempSync(join(tmpdir(), 'd256shell-xfer-'));
  writeFileSync(join(dir, 'отчёт 1.txt'), 'привет\nмир\n');
  writeFileSync(join(dir, 'b.bin'), Buffer.alloc(300_000, 7));

  // upload two files
  await page.getByTestId('upload').click();
  await prompt(page, `${join(dir, 'отчёт 1.txt')}|${join(dir, 'b.bin')}`);
  await expect(row(page, 'отчёт 1.txt')).toBeVisible();
  await expect(row(page, 'b.bin')).toBeVisible();
  await expect(page.getByTestId('xfer-row').first()).toHaveAttribute('data-state', 'done');
  expect((await serverFile(bridge.url, `${work}/отчёт 1.txt`)).text).toBe('привет\nмир\n');
  await page.screenshot({ path: 'screenshots/51-transfers.png' });

  // the same file again: a question, “Keep both” → “report 1 (1).txt”
  await page.getByTestId('upload').click();
  await prompt(page, join(dir, 'отчёт 1.txt'));
  await expect(page.getByTestId('conflict-dialog')).toBeVisible();
  await page.getByTestId('conflict-rename').click();
  await expect(row(page, 'отчёт 1 (1).txt')).toBeVisible();

  // Skip changes nothing
  writeFileSync(join(dir, 'отчёт 1.txt'), 'other');
  await page.getByTestId('upload').click();
  await prompt(page, join(dir, 'отчёт 1.txt'));
  await page.getByTestId('conflict-skip').click();
  expect((await serverFile(bridge.url, `${work}/отчёт 1.txt`)).text).toBe('привет\nмир\n');

  // Replace
  await page.getByTestId('upload').click();
  await prompt(page, join(dir, 'отчёт 1.txt'));
  await page.getByTestId('conflict-replace').click();
  await expect.poll(async () => (await serverFile(bridge.url, `${work}/отчёт 1.txt`)).text).toBe('other');

  // download
  const out = join(dir, 'out');
  mkdirSync(out);
  await row(page, 'b.bin').click();
  await page.getByTestId('download').click();
  await prompt(page, out);
  await expect.poll(() => existsSync(join(out, 'b.bin'))).toBe(true);
  expect(readFileSync(join(out, 'b.bin')).equals(Buffer.alloc(300_000, 7))).toBe(true);
  expect(readdirSync(out).filter((n) => n.includes('.d256shell-'))).toEqual([]);
  // downloading again: conflict, skip
  await page.getByTestId('download').click();
  await prompt(page, out);
  await page.getByTestId('conflict-skip').click();

  // folder up and back
  const tree = join(dir, 'project');
  mkdirSync(join(tree, 'sub'), { recursive: true });
  writeFileSync(join(tree, 'sub', 'f.txt'), 'x');
  await page.getByTestId('upload').click();
  await prompt(page, tree);
  await expect(row(page, 'project')).toBeVisible();
  await row(page, 'project').dblclick();
  await expect(row(page, 'sub')).toBeVisible();
});

test('editor: open, change, Ctrl+S, conflict with differences, CRLF, closing with unsaved changes', async ({ page }) => {
  const work = await open(page);
  const dir = mkdtempSync(join(tmpdir(), 'd256shell-ed-'));
  writeFileSync(join(dir, 'win.txt'), 'one\r\ntwo\r\n');
  writeFileSync(join(dir, 'a.conf'), 'listen 80;\n');
  writeFileSync(join(dir, 'blob.bin'), Buffer.from([1, 2, 0, 3, 255, 0]));
  await page.getByTestId('upload').click();
  await prompt(page, [join(dir, 'win.txt'), join(dir, 'a.conf'), join(dir, 'blob.bin')].join('|'));
  await expect(row(page, 'blob.bin')).toBeVisible();

  // CRLF is kept
  await row(page, 'win.txt').dblclick();
  await expect(page.getByTestId('editor')).toBeVisible();
  await expect(page.getByTestId('ed-info')).toContainText('CRLF');
  await page.locator('.cm-content').click();
  await page.keyboard.press('Control+End');
  await page.keyboard.type('three');
  await expect(page.getByTestId('dirty-dot')).toBeVisible();
  await page.keyboard.press('Control+s');
  await expect(page.getByTestId('ed-status')).toHaveText('Saved');
  await expect(page.getByTestId('dirty-dot')).toHaveCount(0);
  const saved = await serverFile(bridge.url, `${work}/win.txt`);
  expect(saved.eol).toBe('crlf');
  expect(saved.text).toBe('one\ntwo\nthree');
  await page.screenshot({ path: 'screenshots/52-editor.png' });

  // conflict: the file was changed from outside
  await page.keyboard.type(' mine');
  await serverFile(bridge.url, `${work}/win.txt`, 'one\ntwo\nEXTERNAL');
  await page.keyboard.press('Control+s');
  await expect(page.getByTestId('conflict-save-dialog')).toBeVisible();
  await page.getByTestId('cs-diff').click();
  await expect(page.getByTestId('diff-host').locator('.cm-mergeView')).toBeVisible();
  await page.screenshot({ path: 'screenshots/53-conflict.png' });
  await page.getByTestId('cs-cancel').click();
  expect((await serverFile(bridge.url, `${work}/win.txt`)).text).toBe('one\ntwo\nEXTERNAL');
  await page.keyboard.press('Control+s');
  await page.getByTestId('cs-force').click();
  await expect(page.getByTestId('ed-status')).toHaveText('Saved');
  expect((await serverFile(bridge.url, `${work}/win.txt`)).text).toBe('one\ntwo\nthree mine');

  // unsaved changes: closing the tab asks first
  await page.keyboard.type('!');
  await page.getByTestId(`pane-close-f:${work}/win.txt`).click();
  await expect(page.getByTestId('confirm')).toBeVisible();
  await page.getByTestId('confirm-cancel').click();
  await expect(page.getByTestId('editor')).toBeVisible();
  await page.getByTestId(`pane-close-f:${work}/win.txt`).click();
  await page.getByTestId('confirm-ok').click();
  await expect(page.getByTestId('editor')).toHaveCount(0);

  // two files side by side; the binary does not open
  await row(page, 'a.conf').dblclick();
  await expect(page.getByTestId('editor')).toBeVisible();
  await page.getByTestId('pane-term').click();
  await row(page, 'blob.bin').dblclick();
  await expect(page.getByTestId('confirm')).toContainText('binary');
  await page.getByTestId('confirm-cancel').click();
  await expect(page.getByTestId('editor')).toHaveCount(1);
});

test('drag inside the panel: move into a folder with confirmation, delete with a count', async ({ page }) => {
  const work = await open(page);
  const mk = async (kind: 'new-folder' | 'new-file', name: string) => {
    await page.getByTestId(kind).click();
    await prompt(page, name);
  };
  await mk('new-folder', 'target');
  await mk('new-folder', 'src');
  await row(page, 'src').dblclick();
  await mk('new-folder', 'deep');
  await page.getByTestId('up').click();
  await expect(row(page, 'target')).toBeVisible();

  const drag = async () => {
    const from = await row(page, 'src').boundingBox();
    const to = await row(page, 'target').boundingBox();
    await page.mouse.move(from!.x + 30, from!.y + 10);
    await page.mouse.down();
    await page.mouse.move(from!.x + 40, from!.y + 14, { steps: 3 });
    await page.mouse.move(to!.x + 30, to!.y + 10, { steps: 6 });
    await expect(row(page, 'target')).toHaveClass(/drop-target/);
    await page.mouse.up();
  };
  await drag();
  await expect(page.getByTestId('move-confirm')).toContainText(`${work}/target`);
  await page.getByTestId('move-cancel').click();
  await expect(row(page, 'src')).toBeVisible(); // cancel: nothing was moved

  await drag();
  await page.getByTestId('move-ok').click();
  await expect(row(page, 'src')).toHaveCount(0);
  await row(page, 'target').dblclick();
  await expect(row(page, 'src')).toBeVisible();
  await page.getByTestId('up').click();

  // delete: confirmation with the number of objects
  await row(page, 'target').click();
  await page.keyboard.press('Delete');
  await expect(page.getByTestId('confirm')).toContainText('3 folders');
  await page.getByTestId('confirm-ok').click();
  await expect(row(page, 'target')).toHaveCount(0);
});
