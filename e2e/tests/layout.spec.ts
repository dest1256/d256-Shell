import { test, expect } from '@playwright/test';
import { Bridge, startBridge } from '../fixtures';
import { acceptHostKey, addServer, connectFirst, createVault, PASS } from '../helpers';

let bridge: Bridge;
test.afterEach(async () => bridge?.stop());

test('компоновка: заголовок с вкладками, панель, рейка, сессии, строка состояния', async ({ page }) => {
  bridge = await startBridge();
  await page.goto(bridge.url);
  await page.screenshot({ path: 'screenshots/10-setup.png' });
  await page.getByTestId('master1').fill('layout-test-pass');
  await page.getByTestId('master2').fill('layout-test-pass');
  await page.getByTestId('create').click();

  // вкладка «Главная» в заголовке, кнопка «+»
  await expect(page.getByTestId('tab-home')).toBeVisible();
  await expect(page.getByTestId('tab-add')).toBeVisible();
  await expect(page.getByTestId('rail-sessions')).toBeVisible();
  await expect(page.getByTestId('sidebar')).toBeVisible();
  await expect(page.getByTestId('status-count')).toHaveText('Сессий: 0');
  await expect(page.getByTestId('home')).toContainText('Добро пожаловать');

  // добавим две сессии через «+»
  for (const [name, host] of [['Web-01', '203.0.113.1'], ['DB-главный', '203.0.113.2']]) {
    await page.getByTestId('tab-add').click();
    await page.getByTestId('f-name').fill(name);
    await page.getByTestId('f-host').fill(host);
    await page.getByTestId('f-user').fill('deploy');
    await page.getByTestId('f-password').fill('pw');
    await page.getByTestId('form-save').click();
  }
  await expect(page.getByTestId('server-item')).toHaveCount(2);
  await expect(page.getByTestId('home-card')).toHaveCount(2);
  await expect(page.getByTestId('status-count')).toHaveText('Сессий: 2');

  // выбор в списке
  await page.getByTestId('server-item').first().click();
  await expect(page.getByTestId('server-item').first()).toHaveClass(/selected/);
  await page.screenshot({ path: 'screenshots/11-shell.png' });

  // сплиттер меняет ширину боковой панели
  const w0 = (await page.getByTestId('sidebar').boundingBox())!.width;
  const sp = (await page.getByTestId('splitter').boundingBox())!;
  await page.mouse.move(sp.x + 2, sp.y + 100);
  await page.mouse.down();
  await page.mouse.move(sp.x + 82, sp.y + 100, { steps: 5 });
  await page.mouse.up();
  const w1 = (await page.getByTestId('sidebar').boundingBox())!.width;
  expect(w1).toBeGreaterThan(w0 + 60);

  // Ctrl+B / кнопка «Панель» скрывают и показывают боковую панель
  await page.keyboard.press('Control+b');
  await expect(page.getByTestId('sidebar')).toBeHidden();
  await page.getByTestId('toggle-sidebar').click();
  await expect(page.getByTestId('sidebar')).toBeVisible();

  // горизонтального скролла страницы нет
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth > window.innerWidth);
  expect(overflow).toBe(false);

  // узкое окно не ломает раскладку
  await page.setViewportSize({ width: 800, height: 500 });
  await expect(page.getByTestId('tab-home')).toBeVisible();
  await page.screenshot({ path: 'screenshots/12-narrow.png' });
});

test('боковая панель сессий скрывается во вкладке SSH и возвращается на главной', async ({ page }) => {
  const b = await startBridge();
  try {
    await page.goto(b.url);
    await createVault(page);
    await addServer(page, { name: 'Side', host: 'sshd-password', password: PASS });
    await expect(page.getByTestId('sidebar')).toBeVisible();
    await connectFirst(page);
    await acceptHostKey(page);
    await expect(page.getByTestId('tab-s1')).toBeVisible();
    await expect(page.getByTestId('sidebar')).toBeHidden();
    await page.getByTestId('tab-home').click();
    await expect(page.getByTestId('sidebar')).toBeVisible();
  } finally {
    await b.stop();
  }
});
