import { test, expect } from '@playwright/test';
import { Bridge, startBridge } from '../fixtures';
import { acceptHostKey, addServer, connectFirst, createVault, PASS } from '../helpers';

let bridge: Bridge;
test.afterEach(async () => bridge?.stop());

test('monitor: numbers under the terminal, mini charts, detailed window, switching off in settings', async ({ page }) => {
  bridge = await startBridge();
  await page.goto(bridge.url);
  await createVault(page);
  await addServer(page, { name: 'Mon', host: 'sshd-password', password: PASS });
  await connectFirst(page);
  await acceptHostKey(page);
  await expect(page.getByTestId('tab-s1')).toBeVisible();

  const bar = page.getByTestId('monitor-bar');
  await expect(bar).toBeVisible();
  // the numbers appear after the second sample
  await expect(page.getByTestId('mon-cpu')).toContainText(/\d+%/, { timeout: 15_000 });
  await expect(page.getByTestId('mon-mem')).toContainText(/\d+%.*of/);
  await expect(page.getByTestId('mon-net')).toContainText('↓');
  // numbers on the left, charts on the right (after a gap)
  const nums = await page.getByTestId('mon-nums').boundingBox();
  const spark = await bar.locator('canvas').first().boundingBox();
  expect(spark!.x).toBeGreaterThan(nums!.x + nums!.width - 1);
  await page.screenshot({ path: 'screenshots/55-monitor-bar.png' });

  // the terminal is not covered by the bar
  const term = await page.getByTestId('terminal').boundingBox();
  const barBox = await bar.boundingBox();
  expect(term!.y + term!.height).toBeLessThanOrEqual(barBox!.y + 1);

  // a bit of disk load so that the screenshot has non-zero values
  await page.getByTestId('terminal').click();
  await page.keyboard.type('dd if=/dev/zero of=$HOME/ddtest bs=1M count=600 oflag=direct 2>/dev/null; rm -f $HOME/ddtest');
  await page.keyboard.press('Enter');

  // a click opens the detailed window in a tab next to the terminal
  await bar.click();
  await expect(page.getByTestId('monitor')).toBeVisible();
  // like Performance in the Windows task manager: the list of resources on the left, details on the right
  const side = page.getByTestId('md-side');
  await expect(side.getByTestId('md-item-cpu')).toBeVisible();
  await expect(side.getByTestId('md-item-mem')).toBeVisible();
  await expect(side.locator('[data-testid^="md-item-disk-"]').first()).toBeVisible();
  await expect(side.locator('[data-testid^="md-item-net-"]').first()).toBeVisible();
  // CPU: separate charts per core, usage, speed, threads, uptime
  await expect(page.getByTestId('md-title')).toHaveText('CPU');
  await expect(page.locator('.tm-model')).not.toBeEmpty(); // the processor model on the right
  await expect(page.locator('[data-testid="md-cores"] canvas').first()).toBeVisible();
  await expect(page.getByTestId('stat-cpu-util')).toContainText('%');
  await expect(page.getByTestId('stat-cpu-threads')).toContainText(/\d+/);
  await expect(page.getByTestId('stat-cpu-uptime')).toContainText(/\d+:\d\d:\d\d:\d\d/);
  await page.getByTestId('md-cpu-total').click();
  await expect(page.getByTestId('md-chart-cpu')).toBeVisible();
  await page.getByTestId('md-cpu-total').click();
  // refresh rate: pause stops the charts, “every second” resumes them, the choice is saved
  await page.getByTestId('md-rate').selectOption('pause');
  const frozen = await page.getByTestId('stat-cpu-uptime').textContent();
  await page.waitForTimeout(3500);
  expect(await page.getByTestId('stat-cpu-uptime').textContent()).toBe(frozen);
  await page.getByTestId('md-rate').selectOption('1000');
  await expect.poll(() => page.getByTestId('stat-cpu-uptime').textContent(), { timeout: 8000 }).not.toBe(frozen);
  await page.getByTestId('md-rate').selectOption('5000');
  await expect(page.getByTestId('md-rate')).toHaveValue('5000');
  expect(JSON.parse(await page.evaluate(async () => JSON.stringify(await (await fetch('/rpc', { method: 'POST', body: JSON.stringify({ method: 'get_settings', params: {} }) })).json()))).result.monitor_interval_ms).toBe(5000);
  await page.getByTestId('md-rate').selectOption('2000');
  // custom rate in milliseconds (minimum 100 ms): a valid value is applied, an invalid one is rejected
  await page.getByTestId('md-rate').selectOption('custom');
  await page.getByTestId('md-rate-ms').fill('50');
  await page.getByTestId('md-rate-ms').press('Enter');
  await expect(page.getByTestId('md-rate-err')).toContainText('100');
  await page.getByTestId('md-rate-ms').fill('100');
  await page.getByTestId('md-rate-ms').press('Enter');
  await expect(page.getByTestId('md-rate-err')).toHaveText('');
  await expect.poll(async () => JSON.parse(await page.evaluate(async () => JSON.stringify(await (await fetch('/rpc', { method: 'POST', body: JSON.stringify({ method: 'get_settings', params: {} }) })).json()))).result.monitor_interval_ms).toBe(100);
  const before = await page.getByTestId('stat-cpu-uptime').textContent();
  await expect.poll(() => page.getByTestId('stat-cpu-uptime').textContent(), { timeout: 5000 }).not.toBe(before);
  await page.waitForTimeout(1500); // ~15 samples: the interface must not hang
  await expect(page.getByTestId('stat-cpu-util')).toContainText('%');
  await page.getByTestId('md-rate').selectOption('2000');
  await page.waitForTimeout(4500); // collect a few points
  await page.screenshot({ path: 'screenshots/56-monitor-detail.png' });
  // memory
  await side.getByTestId('md-item-mem').click();
  await expect(page.getByTestId('md-title')).toHaveText('Memory');
  await expect(page.getByTestId('stat-mem-used')).toContainText('%');
  await expect(page.getByTestId('stat-mem-avail')).toContainText(/GB|MB/);
  await expect(page.getByTestId('md-membar')).toBeVisible();
  await page.screenshot({ path: 'screenshots/65-monitor-memory.png' });
  // disk: active time, response time, speeds, capacity and type from /sys
  await side.locator('[data-testid^="md-item-disk-"]').first().click();
  await expect(page.getByTestId('md-title')).toContainText('Disk');
  await expect(page.getByTestId('stat-disk-active')).toContainText('%');
  await expect(page.getByTestId('stat-disk-resp')).toContainText('ms');
  await expect(page.getByTestId('stat-disk-read')).toContainText(/(B|KB|MB)\/s/);
  // capacity, type and model were really read from /sys (not “—”)
  const val = (k: string) => page.locator('.tm-tk', { hasText: k }).locator('+ .tm-tv');
  await expect(val('Capacity')).toHaveText(/\d.*(MB|GB|TB)/);
  await expect(val('Type')).toHaveText(/HDD|SSD/);
  await expect(val('Model')).not.toHaveText('—');
  await page.screenshot({ path: 'screenshots/66-monitor-disk.png' });
  // network: send and receive in bit/s
  await side.locator('[data-testid^="md-item-net-"]').first().click();
  await expect(page.getByTestId('stat-net-send')).toContainText('bit/s');
  await expect(page.getByTestId('stat-net-recv')).toContainText('bit/s');
  await page.screenshot({ path: 'screenshots/67-monitor-net.png' });
  // the charts are really drawn (not an empty canvas)
  const painted = await page.locator('[data-testid^="md-chart-net-"]').first().evaluate((c: HTMLCanvasElement) => {
    const d = c.getContext('2d')!.getImageData(0, 0, c.width, c.height).data;
    for (let i = 3; i < d.length; i += 4) if (d[i] > 0) return true;
    return false;
  });
  expect(painted).toBe(true);
  await page.getByTestId('pane-close-monitor').click();
  await expect(page.getByTestId('monitor')).toHaveCount(0);

  // switching off in settings hides the panel
  await page.getByRole('button', { name: 'Settings' }).click();
  await page.getByTestId('set-mon').uncheck();
  await page.getByTestId('settings-save').click();
  await expect(bar).toBeHidden();
});

test('monitor: server without SFTP: a clear message, the terminal works', async ({ page }) => {
  bridge = await startBridge();
  await page.goto(bridge.url);
  await createVault(page);
  await addServer(page, { name: 'NoSftp', host: 'sshd-nosftp', password: PASS });
  await connectFirst(page);
  await acceptHostKey(page);
  await expect(page.getByTestId('mon-off')).toContainText('no SFTP');
});
