import { test, expect, Page } from '@playwright/test';
import { chmodSync, mkdirSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { Bridge, startBridge } from '../fixtures';
import { acceptHostKey, addServer, connectFirst, createVault, PASS } from '../helpers';

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

test('local monitor: the column to the right of the terminal (chips on top, details below), foldable', async ({ page }) => {
  bridge = await startBridge(undefined, { D256SHELL_NVIDIA_SMI: fakeSmi() });
  await page.goto(bridge.url);
  await createVault(page);
  await openLocal(page);
  await expect(page.getByTestId('mon-cpu')).toContainText(/\d+%/, { timeout: 15_000 });
  const col = page.getByTestId('mon-col');
  await expect(col).toBeHidden(); // folded by default
  await page.getByTestId('mon-col-toggle').click();
  await expect(col).toBeVisible();
  // the column is to the right of the terminal, the resources are chips at the top, the details are one below the other
  const t = (await page.getByTestId('local-terminal').boundingBox())!;
  const c = (await col.boundingBox())!;
  expect(c.x).toBeGreaterThanOrEqual(t.x + t.width - 1);
  // measured in one go: the chips are rebuilt with every sample
  const geo = await col.evaluate((el) => {
    const chips = [...el.querySelectorAll('.tm-side .tm-item')].map((c) => c.getBoundingClientRect());
    return { n: chips.length, firstY: chips[0]?.y ?? 0, secondY: chips[1]?.y ?? 0, h: chips[0]?.height ?? 0, mainY: el.querySelector('.tm-main')!.getBoundingClientRect().y };
  });
  expect(geo.n).toBeGreaterThanOrEqual(4);
  expect(Math.abs(geo.firstY - geo.secondY)).toBeLessThan(geo.h); // chips share a row: not a vertical list
  expect(geo.mainY).toBeGreaterThan(geo.firstY);
  await col.getByTestId('md-item-gpu-0').click({ force: true });
  await expect(col.getByTestId('stat-gpu-3d')).toHaveText('92%');
  await expect(col.getByTestId('stat-gpu-cuda')).toBeVisible();
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

test('local monitor: without a video card tool nothing about GPU is shown, the rest works', async ({ page }) => {
  bridge = await startBridge(undefined, { D256SHELL_NVIDIA_SMI: '/definitely/not/here' });
  await page.goto(bridge.url);
  await createVault(page);
  await openLocal(page);
  await expect(page.getByTestId('mon-cpu')).toContainText(/\d+%/, { timeout: 15_000 });
  await expect(page.getByTestId('mon-gpu3d')).toHaveCount(0);
  await page.getByTestId('mon-col-toggle').click();
  await expect(page.getByTestId('mon-col').locator('[data-testid^="md-item-gpu"]')).toHaveCount(0);
  await expect(page.getByTestId('mon-col').getByTestId('md-item-cpu')).toBeVisible();
});

test('server monitor: an SSH tab has the same column (no video card of a server)', async ({ page }) => {
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
  await expect(col.getByTestId('md-item-cpu')).toBeVisible();
  await expect(col.getByTestId('md-item-mem')).toBeVisible();
  await expect(col.locator('[data-testid^="md-item-gpu"]')).toHaveCount(0);
  await expect(col.getByTestId('md-item-procs')).toHaveCount(0);
  await page.screenshot({ path: 'screenshots/85-ssh-monitor-column.png' });
});
