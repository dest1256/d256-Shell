import { test, expect } from '@playwright/test';
import { mkdtempSync, rmSync, writeFileSync, readFileSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { Bridge, startBridge } from '../fixtures';

const MASTER = 'correct horse battery';
let dir: string;
let bridge: Bridge;

test.beforeAll(() => {
  dir = mkdtempSync(join(tmpdir(), 'd256shell-e2e-vault-'));
});
test.afterAll(() => rmSync(dir, { recursive: true, force: true }));
test.afterEach(async () => bridge?.stop());

test('создание мастер-пароля, сервер, перезапуск и разблокировка', async ({ page }) => {
  bridge = await startBridge(dir);
  await page.goto(bridge.url);

  // создание: пароли должны совпадать и быть не короче 8 символов
  await page.getByTestId('master1').fill(MASTER);
  await page.getByTestId('master2').fill('другой пароль');
  await page.getByTestId('create').click();
  await expect(page.getByTestId('error')).toContainText('не совпадают');
  await page.getByTestId('master2').fill(MASTER);
  await page.getByTestId('create').click();
  await expect(page.getByTestId('server-list')).toContainText('Сессий пока нет');
  await page.screenshot({ path: 'screenshots/01-empty-list.png' });

  // добавление сервера
  await page.getByTestId('add').click();
  await page.getByTestId('f-name').fill('Прод сервер');
  await page.getByTestId('f-host').fill('-oProxyCommand=evil');
  await page.getByTestId('f-user').fill('root');
  await page.getByTestId('form-save').click();
  await expect(page.getByTestId('form-error')).toContainText('Некорректный адрес'); // защита от инъекции опций
  await page.getByTestId('f-host').fill('203.0.113.10');
  await page.getByTestId('f-port').fill('2200');
  await page.getByTestId('f-password').fill('S3cret-пароль');
  await page.screenshot({ path: 'screenshots/02-server-form.png' });
  await page.getByTestId('form-save').click();
  const item = page.getByTestId('server-item');
  await expect(item).toContainText('Прод сервер');
  await expect(item).toContainText('root@203.0.113.10:2200');

  // пароль виден только по «глазу» и совпадает с введённым
  await item.hover();
  await item.getByTestId('edit').click();
  await expect(page.getByTestId('f-password')).toHaveAttribute('type', 'password');
  await page.getByTestId('eye').click();
  await expect(page.getByTestId('f-password')).toHaveValue('S3cret-пароль');
  await page.getByTestId('form-cancel').click();

  // поиск
  await page.getByTestId('search').fill('нет-такого');
  await expect(page.getByTestId('server-list')).toContainText('Ничего не найдено');
  await page.getByTestId('search').fill('203.0.113');
  await expect(page.getByTestId('server-item')).toHaveCount(1);

  // файл хранилища не содержит открытого текста
  const raw = readFileSync(join(dir, 'vault.bin'));
  expect(raw.includes('S3cret')).toBe(false);
  expect(raw.includes('203.0.113.10')).toBe(false);

  // перезапуск: разблокировка
  await bridge.stop();
  bridge = await startBridge(dir);
  await page.goto(bridge.url);
  await page.getByTestId('master').fill('неверный пароль');
  await page.getByTestId('unlock').click();
  await expect(page.getByTestId('error')).toContainText('Неверный мастер-пароль');
  await page.screenshot({ path: 'screenshots/03-unlock-error.png' });
  await page.getByTestId('master').fill(MASTER);
  await page.getByTestId('unlock').click();
  await expect(page.getByTestId('server-item')).toContainText('Прод сервер');

  // дублирование и удаление с подтверждением
  await page.getByTestId('server-item').first().hover();
  await page.getByTestId('dup').first().click();
  await expect(page.getByTestId('server-item')).toHaveCount(2);
  await page.getByTestId('server-item').nth(1).hover();
  await page.getByTestId('delete').nth(1).click();
  await page.getByTestId('confirm-cancel').click();
  await expect(page.getByTestId('server-item')).toHaveCount(2);
  await page.getByTestId('server-item').nth(1).hover();
  await page.getByTestId('delete').nth(1).click();
  await page.getByTestId('confirm-ok').click();
  await expect(page.getByTestId('server-item')).toHaveCount(1);
  await page.screenshot({ path: 'screenshots/04-main.png' });

  // смена мастер-пароля
  await page.getByTestId('change-master').click();
  await page.getByTestId('cm-old').fill('не тот');
  await page.getByTestId('cm-new1').fill('new master password');
  await page.getByTestId('cm-new2').fill('new master password');
  await page.getByTestId('cm-ok').click();
  await expect(page.getByTestId('cm-error')).toContainText('Неверный мастер-пароль');
  await page.getByTestId('cm-old').fill(MASTER);
  await page.getByTestId('cm-ok').click();
  await page.getByTestId('lock').click();
  await page.getByTestId('master').fill(MASTER);
  await page.getByTestId('unlock').click();
  await expect(page.getByTestId('error')).toContainText('Неверный мастер-пароль');
  await page.getByTestId('master').fill('new master password');
  await page.getByTestId('unlock').click();
  await expect(page.getByTestId('server-item')).toHaveCount(1);
});

test('повреждённый vault.bin: восстановление из .bak', async ({ page }) => {
  expect(existsSync(join(dir, 'vault.bin.bak'))).toBe(true); // создан предыдущими сохранениями
  writeFileSync(join(dir, 'vault.bin'), 'garbage');
  bridge = await startBridge(dir);
  await page.goto(bridge.url);
  // .bak хранит версию до смены мастер-пароля, поэтому подходит старый пароль
  await page.getByTestId('master').fill(MASTER);
  await page.getByTestId('unlock').click();
  await expect(page.getByTestId('error')).toContainText('повреждён');
  await page.screenshot({ path: 'screenshots/05-corrupt.png' });
  await page.getByTestId('restore').click();
  await expect(page.getByTestId('server-list')).toBeVisible();
});
