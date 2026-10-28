import { test, expect } from '@playwright/test';
import { Bridge, startBridge } from '../fixtures';
import { acceptHostKey, addServer, connectFirst, createVault, PASS } from '../helpers';

let bridge: Bridge;
test.afterEach(async () => bridge?.stop());

test('монитор: цифры под терминалом, мини-графики, подробное окно, отключение в настройках', async ({ page }) => {
  bridge = await startBridge();
  await page.goto(bridge.url);
  await createVault(page);
  await addServer(page, { name: 'Mon', host: 'sshd-password', password: PASS });
  await connectFirst(page);
  await acceptHostKey(page);
  await expect(page.getByTestId('tab-s1')).toBeVisible();

  const bar = page.getByTestId('monitor-bar');
  await expect(bar).toBeVisible();
  // цифры появляются после второго замера
  await expect(page.getByTestId('mon-cpu')).toContainText(/\d+%/, { timeout: 15_000 });
  await expect(page.getByTestId('mon-mem')).toContainText(/\d+%.*из/);
  await expect(page.getByTestId('mon-net')).toContainText('↓');
  // цифры слева, графики правее (после промежутка)
  const nums = await page.getByTestId('mon-nums').boundingBox();
  const spark = await bar.locator('canvas').first().boundingBox();
  expect(spark!.x).toBeGreaterThan(nums!.x + nums!.width - 1);
  await page.screenshot({ path: 'screenshots/55-monitor-bar.png' });

  // терминал не перекрыт панелью
  const term = await page.getByTestId('terminal').boundingBox();
  const barBox = await bar.boundingBox();
  expect(term!.y + term!.height).toBeLessThanOrEqual(barBox!.y + 1);

  // немного нагрузки на диск, чтобы на скриншоте были ненулевые значения
  await page.getByTestId('terminal').click();
  await page.keyboard.type('dd if=/dev/zero of=$HOME/ddtest bs=1M count=600 oflag=direct 2>/dev/null; rm -f $HOME/ddtest');
  await page.keyboard.press('Enter');

  // клик открывает подробное окно во вкладке рядом с терминалом
  await bar.click();
  await expect(page.getByTestId('monitor')).toBeVisible();
  // как «Производительность» в диспетчере задач Windows: слева список ресурсов, справа подробности
  const side = page.getByTestId('md-side');
  await expect(side.getByTestId('md-item-cpu')).toBeVisible();
  await expect(side.getByTestId('md-item-mem')).toBeVisible();
  await expect(side.locator('[data-testid^="md-item-disk-"]').first()).toBeVisible();
  await expect(side.locator('[data-testid^="md-item-net-"]').first()).toBeVisible();
  // ЦП: отдельные диаграммы по ядрам, использование, скорость, потоки, время работы
  await expect(page.getByTestId('md-title')).toHaveText('ЦП');
  await expect(page.locator('.tm-model')).not.toBeEmpty(); // модель процессора справа
  await expect(page.locator('[data-testid="md-cores"] canvas').first()).toBeVisible();
  await expect(page.getByTestId('stat-cpu-util')).toContainText('%');
  await expect(page.getByTestId('stat-cpu-threads')).toContainText(/\d+/);
  await expect(page.getByTestId('stat-cpu-uptime')).toContainText(/\d+:\d\d:\d\d:\d\d/);
  await page.getByTestId('md-cpu-total').click();
  await expect(page.getByTestId('md-chart-cpu')).toBeVisible();
  await page.getByTestId('md-cpu-total').click();
  // частота обновления: пауза останавливает графики, «каждую секунду» возобновляет, выбор сохраняется
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
  // своя частота в миллисекундах (минимум 100 мс): допустимое значение применяется, недопустимое отклоняется
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
  await page.waitForTimeout(1500); // ~15 выборок: интерфейс не должен зависнуть
  await expect(page.getByTestId('stat-cpu-util')).toContainText('%');
  await page.getByTestId('md-rate').selectOption('2000');
  await page.waitForTimeout(4500); // накопить несколько точек
  await page.screenshot({ path: 'screenshots/56-monitor-detail.png' });
  // память
  await side.getByTestId('md-item-mem').click();
  await expect(page.getByTestId('md-title')).toHaveText('Память');
  await expect(page.getByTestId('stat-mem-used')).toContainText('%');
  await expect(page.getByTestId('stat-mem-avail')).toContainText(/ГБ|МБ/);
  await expect(page.getByTestId('md-membar')).toBeVisible();
  await page.screenshot({ path: 'screenshots/65-monitor-memory.png' });
  // диск: активное время, время ответа, скорости, ёмкость и тип из /sys
  await side.locator('[data-testid^="md-item-disk-"]').first().click();
  await expect(page.getByTestId('md-title')).toContainText('Диск');
  await expect(page.getByTestId('stat-disk-active')).toContainText('%');
  await expect(page.getByTestId('stat-disk-resp')).toContainText('мс');
  await expect(page.getByTestId('stat-disk-read')).toContainText(/(Б|КБ|МБ)\/с/);
  // ёмкость, тип и модель реально прочитаны из /sys (а не «—»)
  const val = (k: string) => page.locator('.tm-tk', { hasText: k }).locator('+ .tm-tv');
  await expect(val('Ёмкость')).toHaveText(/\d.*(МБ|ГБ|ТБ)/);
  await expect(val('Тип')).toHaveText(/HDD|SSD/);
  await expect(val('Модель')).not.toHaveText('—');
  await page.screenshot({ path: 'screenshots/66-monitor-disk.png' });
  // сеть: отправка и получение в бит/с
  await side.locator('[data-testid^="md-item-net-"]').first().click();
  await expect(page.getByTestId('stat-net-send')).toContainText('бит/с');
  await expect(page.getByTestId('stat-net-recv')).toContainText('бит/с');
  await page.screenshot({ path: 'screenshots/67-monitor-net.png' });
  // графики реально нарисованы (не пустой холст)
  const painted = await page.locator('[data-testid^="md-chart-net-"]').first().evaluate((c: HTMLCanvasElement) => {
    const d = c.getContext('2d')!.getImageData(0, 0, c.width, c.height).data;
    for (let i = 3; i < d.length; i += 4) if (d[i] > 0) return true;
    return false;
  });
  expect(painted).toBe(true);
  await page.getByTestId('pane-close-monitor').click();
  await expect(page.getByTestId('monitor')).toHaveCount(0);

  // выключение в настройках прячет панель
  await page.getByRole('button', { name: 'Настройки' }).click();
  await page.getByTestId('set-mon').uncheck();
  await page.getByTestId('settings-save').click();
  await expect(bar).toBeHidden();
});

test('монитор: сервер без SFTP — понятное сообщение, терминал работает', async ({ page }) => {
  bridge = await startBridge();
  await page.goto(bridge.url);
  await createVault(page);
  await addServer(page, { name: 'NoSftp', host: 'sshd-nosftp', password: PASS });
  await connectFirst(page);
  await acceptHostKey(page);
  await expect(page.getByTestId('mon-off')).toContainText('нет SFTP');
});
