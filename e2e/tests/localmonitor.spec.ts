import { test, expect, Page } from '@playwright/test';
import { chmodSync, mkdirSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { Bridge, startBridge } from '../fixtures';
import { acceptHostKey, addServer, connectFirst, createVault, PASS, rpc } from '../helpers';

let bridge: Bridge;
test.afterEach(async () => bridge?.stop());

/** A stand-in for nvidia-smi (a real card is not available in the test container). */
function fakeSmi(): string {
  const dir = join(tmpdir(), `d256shell-e2e-smi-${Date.now()}`);
  mkdirSync(dir, { recursive: true });
  const p = join(dir, 'nvidia-smi');
  writeFileSync(p, "#!/bin/sh\ncase \"$1\" in\n  --query-gpu*) echo '0, NVIDIA GeForce RTX 4090, 92, 21914, 24564, 71, 312.45, 450.00, 64, 2535';;\n  --query-compute-apps*) echo '1, 20800';;\nesac\n");
  chmodSync(p, 0o755);
  return p;
}

async function openLocal(page: Page) {
  await page.getByTestId('rail-terminals').click();
  await page.getByTestId('profile-item').first().dblclick();
  await expect(page.getByTestId('tab-local1')).toBeVisible();
}

test('local monitor: a bar under the local terminal with the video card, a click opens the monitor tab', async ({ page }) => {
  bridge = await startBridge(undefined, { D256SHELL_NVIDIA_SMI: fakeSmi() });
  await page.goto(bridge.url);
  await createVault(page);
  await openLocal(page);
  const bar = page.getByTestId('monitor-bar');
  await expect(bar).toBeVisible();
  await expect(page.getByTestId('mon-cpu')).toContainText(/\d+%/, { timeout: 15_000 });
  await expect(page.getByTestId('mon-mem')).toContainText(/of/);
  // the video card: 3D load, memory, temperature and power from nvidia-smi
  await expect(page.getByTestId('mon-gpu3d')).toContainText('92%');
  await expect(page.getByTestId('mon-vram')).toContainText('21.4 GB');
  await expect(page.getByTestId('mon-gputemp')).toContainText('71 °C');
  await expect(page.getByTestId('mon-gputemp')).toContainText('312 W');
  await page.screenshot({ path: 'screenshots/82-local-monitor-bar.png' });
  // the terminal is not covered by the bar
  const term = (await page.getByTestId('local-terminal').boundingBox())!;
  const barBox = (await bar.boundingBox())!;
  expect(term.y + term.height).toBeLessThanOrEqual(barBox.y + 1);

  // a click opens the detailed tab (the same task manager as for servers) with the video card and the processes
  await bar.click({ position: { x: 20, y: 5 } });
  const tab = page.getByTestId('monitor'); // the full tab (the folded column has the same test ids)
  await expect(tab).toBeVisible();
  await expect(tab.getByTestId('md-item-gpu-0')).toBeVisible();
  await tab.getByTestId('md-item-gpu-0').click();
  await expect(tab.getByTestId('md-title')).toHaveText('GPU');
  await expect(tab.getByTestId('stat-gpu-3d')).toHaveText('92%');
  await expect(tab.getByTestId('stat-gpu-vram')).toContainText('21.4 GB / 24');
  await expect(tab.getByTestId('stat-gpu-temp')).toHaveText('71 °C');
  await expect(tab.getByTestId('stat-gpu-power')).toHaveText('312 W');
  await page.screenshot({ path: 'screenshots/83-local-monitor-gpu.png' });
  await tab.getByTestId('md-item-procs').click();
  await expect(tab.getByTestId('md-proc').first()).toBeVisible({ timeout: 15_000 });
});

test('local monitor: the column shows the ticked indicators one below the other, foldable', async ({ page }) => {
  bridge = await startBridge(undefined, { D256SHELL_NVIDIA_SMI: fakeSmi() });
  await page.goto(bridge.url);
  await createVault(page);
  await openLocal(page);
  await expect(page.getByTestId('mon-cpu')).toContainText(/\d+%/, { timeout: 15_000 });
  const col = page.getByTestId('mon-col');
  await expect(col).toBeHidden(); // folded by default
  await page.getByTestId('mon-col-toggle').click();
  await expect(col).toBeVisible();
  // the column is to the right of the terminal; the default set of this computer is stacked, not switched
  const t = (await page.getByTestId('local-terminal').boundingBox())!;
  const c = (await col.boundingBox())!;
  expect(c.x).toBeGreaterThanOrEqual(t.x + t.width - 1);
  const tile = (id: string) => col.locator(`[data-tile="${id}"]`);
  await expect(tile('cpu')).toBeVisible({ timeout: 15_000 });
  await expect(tile('mem')).toBeVisible();
  await expect(tile('gpu3d:0')).toBeVisible();
  await expect(tile('gpu3d:0').locator('.mt-v')).toHaveText('92%');
  await expect(tile('vram:0').locator('.mt-v')).toContainText('21.4 GB / 24');
  await expect(tile('net')).toHaveCount(0); // not in the default set of this computer
  const ys = await col.evaluate((el) => [...el.querySelectorAll('[data-testid="mon-tile"]')].map((e) => e.getBoundingClientRect().y));
  expect(ys.length).toBeGreaterThanOrEqual(4);
  expect(ys.every((y, i) => i === 0 || y > ys[i - 1])).toBe(true);
  await page.screenshot({ path: 'screenshots/84-monitor-column.png' });

  // the splitter changes the width; the button folds and unfolds
  const w0 = c.width;
  const sp = (await page.getByTestId('mon-col-splitter').boundingBox())!;
  await page.mouse.move(sp.x + 2, sp.y + 200);
  await page.mouse.down();
  await page.mouse.move(sp.x - 78, sp.y + 200, { steps: 5 });
  await page.mouse.up();
  expect((await col.boundingBox())!.width).toBeGreaterThan(w0 + 50);
  await page.getByTestId('mon-col-toggle').click();
  await expect(col).toBeHidden();
  await page.getByTestId('mon-col-toggle').click();
  await expect(col).toBeVisible();
});

test('local monitor: the indicators are chosen with ticks and remembered', async ({ page }) => {
  bridge = await startBridge(undefined, { D256SHELL_NVIDIA_SMI: fakeSmi() });
  await page.goto(bridge.url);
  await createVault(page);
  await openLocal(page);
  await page.getByTestId('mon-col-toggle').click();
  const col = page.getByTestId('mon-col');
  const tile = (id: string) => col.locator(`[data-tile="${id}"]`);
  await expect(tile('cpu')).toBeVisible({ timeout: 15_000 });
  // only RAM + 3D (+ the network); CUDA is not offered where only nvidia-smi is the source (Linux)
  await col.getByTestId('mon-col-pick').click();
  await expect(col.getByTestId('mon-pick-net')).toBeVisible();
  await col.getByTestId('mon-pick-cpu').uncheck();
  await col.getByTestId('mon-pick-vram').uncheck();
  await col.getByTestId('mon-pick-net').check();
  await expect(tile('cpu')).toHaveCount(0);
  await expect(tile('vram:0')).toHaveCount(0);
  await expect(tile('net')).toBeVisible();
  await expect(tile('mem')).toBeVisible();
  await expect(tile('gpu3d:0')).toBeVisible();
  await expect(col.getByTestId('mon-pick-cuda')).toHaveCount(0);
  await expect.poll(async () => ((await rpc(bridge.url, 'get_settings')) as { monitor_cols: Record<string, string[]> }).monitor_cols['local']).toEqual(['mem', 'gpu3d', 'cuda', 'net']) // cuda stays ticked, it only is not offered here;
  // a new page load (a new connection to the same data) brings the same choice back
  await page.reload();
  await openLocal(page);
  await expect(page.getByTestId('mon-col')).toBeVisible();
  await expect(page.getByTestId('mon-col').locator('[data-tile="net"]')).toBeVisible({ timeout: 15_000 });
  await expect(page.getByTestId('mon-col').locator('[data-tile="cpu"]')).toHaveCount(0);
  // "default set" brings the defaults back
  await page.getByTestId('mon-col').getByTestId('mon-col-pick').click();
  await page.getByTestId('mon-pick-reset').click();
  await expect(page.getByTestId('mon-col').locator('[data-tile="cpu"]')).toBeVisible();
  await expect(page.getByTestId('mon-col').locator('[data-tile="net"]')).toHaveCount(0);
});

test('local monitor: without a video card tool nothing about GPU is offered, the rest works', async ({ page }) => {
  bridge = await startBridge(undefined, { D256SHELL_NVIDIA_SMI: '/definitely/not/here' });
  await page.goto(bridge.url);
  await createVault(page);
  await openLocal(page);
  await expect(page.getByTestId('mon-cpu')).toContainText(/\d+%/, { timeout: 15_000 });
  await expect(page.getByTestId('mon-gpu3d')).toHaveCount(0);
  await page.getByTestId('mon-col-toggle').click();
  const col = page.getByTestId('mon-col');
  await expect(col.locator('[data-tile="cpu"]')).toBeVisible({ timeout: 15_000 });
  await expect(col.locator('[data-tile^="gpu3d"]')).toHaveCount(0);
  await col.getByTestId('mon-col-pick').click();
  await expect(col.getByTestId('mon-pick-mem')).toBeVisible();
  await expect(col.getByTestId('mon-pick-gpu3d')).toHaveCount(0);
});

test('local monitor: tabs share one monitor (a second tab already has data)', async ({ page }) => {
  bridge = await startBridge();
  await page.goto(bridge.url);
  await createVault(page);
  await openLocal(page);
  await expect(page.getByTestId('mon-cpu')).toContainText(/\d+%/, { timeout: 15_000 });
  await page.waitForTimeout(2500); // some history builds up
  // a second terminal: its bar shows numbers at once; a monitor of its own would need a couple of seconds
  await page.getByTestId('tab-home').click();
  await page.getByTestId('profile-item').first().dblclick();
  await expect(page.getByTestId('tab-local2')).toBeVisible();
  await expect(page.getByTestId('mon-cpu').filter({ visible: true })).toContainText(/\d+%/, { timeout: 1200 });
});

test('server monitor: the column of an SSH tab follows the chosen target, this computer or the server', async ({ page }) => {
  bridge = await startBridge();
  await page.goto(bridge.url);
  await createVault(page);
  await addServer(page, { name: 'Col', host: 'sshd-password', password: PASS });
  await connectFirst(page);
  await acceptHostKey(page);
  await expect(page.getByTestId('mon-cpu')).toContainText(/\d+%/, { timeout: 15_000 });
  await page.getByTestId('mon-col-toggle').click();
  const col = page.getByTestId('mon-col');
  await expect(col).toBeVisible();
  // the column watches the server of this tab: its default set; no video card, no process list
  const tile = (id: string) => col.locator(`[data-tile="${id}"]`);
  await expect(tile('cpu')).toBeVisible({ timeout: 15_000 });
  await expect(tile('mem')).toBeVisible();
  await expect(tile('net')).toBeVisible();
  await expect(col.locator('[data-tile^="gpu"]')).toHaveCount(0);
  await expect(col.getByTestId('mon-col-target')).toHaveValue(/server:/);
  await expect(col.getByTestId('mon-col-target').locator('option')).toHaveText(['This computer', 'Col']);
  await page.screenshot({ path: 'screenshots/85-ssh-monitor-column.png' });
  // switch to this computer while staying in the SSH tab: the choice is stored, the indicators are those of this computer
  await col.getByTestId('mon-col-target').selectOption('local');
  await expect(tile('cpu')).toBeVisible();
  await expect.poll(async () => ((await rpc(bridge.url, 'get_settings')) as { monitor_col_target: string }).monitor_col_target).toBe('local');
  // the bar under the terminal still shows the server of the tab
  await expect(page.getByTestId('monitor-bar')).toBeVisible();
  // back to the server of the tab
  await col.getByTestId('mon-col-target').selectOption({ index: 1 });
  await expect.poll(async () => ((await rpc(bridge.url, 'get_settings')) as { monitor_col_target: string }).monitor_col_target).toBe('auto');
});

test('server monitor: two tabs of one server share one monitor, closing the first keeps it for the second', async ({ page }) => {
  bridge = await startBridge();
  await page.goto(bridge.url);
  await createVault(page);
  await addServer(page, { name: 'Twice', host: 'sshd-password', password: PASS });
  await connectFirst(page);
  await acceptHostKey(page);
  await expect(page.getByTestId('mon-cpu')).toContainText(/\d+%/, { timeout: 15_000 });
  await page.waitForTimeout(2500);
  await page.getByTestId('tab-home').click();
  await connectFirst(page);
  await expect(page.getByTestId('tab-s2')).toBeVisible();
  // the second tab shows numbers at once: the monitor of the server already exists
  await expect(page.getByTestId('mon-cpu').filter({ visible: true })).toContainText(/\d+%/, { timeout: 1200 });
  // one target in the list, not two
  const vis = (id: string) => page.getByTestId(id).filter({ visible: true });
  await vis('mon-col-toggle').click();
  await expect(vis('mon-col').getByTestId('mon-col-target').locator('option')).toHaveText(['This computer', 'Twice']);
  await page.getByTestId('tab-close-s1').click();
  await page.getByTestId('confirm-ok').click();
  await expect(vis('mon-col').getByTestId('mon-col-target').locator('option')).toHaveText(['This computer', 'Twice']);
  await expect(vis('mon-col').locator('[data-tile="cpu"]')).toBeVisible({ timeout: 15_000 });
  // closing the last tab withdraws the server from the list
  await page.getByTestId('tab-close-s2').click();
  await page.getByTestId('confirm-ok').click();
  await expect(page.getByTestId('tab-s2')).toHaveCount(0);
});

test('server monitor: only the parts on screen are asked of the server; a second tab shares the connection', async ({ page }) => {
  bridge = await startBridge();
  const asked: { want?: string[]; session_id: string }[] = [];
  page.on('request', (r) => {
    const b = r.postData();
    if (!b || !b.includes('"monitor_sample"')) return;
    asked.push((JSON.parse(b) as { params: { want?: string[]; session_id: string } }).params);
  });
  await page.goto(bridge.url);
  await createVault(page);
  await addServer(page, { name: 'Lean', host: 'sshd-password', password: PASS });
  await connectFirst(page);
  await acceptHostKey(page);
  await expect(page.getByTestId('mon-cpu')).toContainText(/\d+%/, { timeout: 15_000 });
  await expect.poll(() => asked.length).toBeGreaterThan(2);
  // the bar shows processor, memory, load, uptime and network: the disks are not read
  const bar = asked[asked.length - 1].want!;
  expect([...bar].sort()).toEqual(['cpu', 'load', 'mem', 'net', 'uptime']);
  // the full Monitor tab needs everything: no restriction is sent
  await page.getByTestId('monitor-bar').click({ position: { x: 20, y: 5 } });
  const mark = asked.length;
  await expect.poll(() => asked.slice(mark).some((a) => a.want === undefined), { timeout: 15_000 }).toBe(true);
  // a second tab of the same server shares the connection: both tabs have the same session to ask through
  await page.getByTestId('tab-home').click();
  await connectFirst(page);
  await expect(page.getByTestId('tab-s2')).toBeVisible();
  await expect(page.getByTestId('mon-cpu').filter({ visible: true })).toContainText(/\d+%/, { timeout: 5000 });
});
