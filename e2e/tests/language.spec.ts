import { test, expect } from '@playwright/test';
import { Bridge, startBridge } from '../fixtures';
import { createVault } from '../helpers';

let bridge: Bridge;
test.afterEach(async () => bridge?.stop());

test('language: the interface is English by default, the choice in Settings is applied and remembered', async ({ page }) => {
  bridge = await startBridge();
  await page.goto(bridge.url);
  await createVault(page);
  await expect(page.getByRole('button', { name: 'Settings' })).toBeVisible();

  // every supported language switches the toolbar; the setting is saved in settings.json
  const expected: Record<string, string> = { ru: 'Настройки', es: 'Ajustes', de: 'Einstellungen', fr: 'Paramètres', 'pt-BR': 'Configurações', 'zh-CN': '设置', ja: '設定' };
  let back = 'Settings';
  for (const [code, word] of Object.entries(expected)) {
    await page.getByRole('button', { name: back }).click();
    await page.getByTestId('set-lang').selectOption(code);
    await page.getByTestId('settings-save').click();
    await expect(page.getByRole('button', { name: word })).toBeVisible({ timeout: 10_000 }); // the page reloads in the new language
    expect(await page.evaluate(() => document.documentElement.lang)).toBe(code);
    back = word;
  }
  const saved = await page.evaluate(async () => (await (await fetch('/rpc', { method: 'POST', body: JSON.stringify({ method: 'get_settings', params: {} }) })).json()).result.language);
  expect(saved).toBe('ja');

  // back to English
  await page.getByRole('button', { name: back }).click();
  await page.getByTestId('set-lang').selectOption('en');
  await page.getByTestId('settings-save').click();
  await expect(page.getByRole('button', { name: 'Settings' })).toBeVisible({ timeout: 10_000 });
  await page.screenshot({ path: 'screenshots/70-language-en.png' });
});

test('language: messages from the core are translated too', async ({ page }) => {
  bridge = await startBridge();
  await page.goto(bridge.url);
  await createVault(page, 'e2e-master-pass');
  await page.getByRole('button', { name: 'Settings' }).click();
  await page.getByTestId('set-lang').selectOption('ru');
  await page.getByTestId('settings-save').click();
  await expect(page.getByRole('button', { name: 'Настройки' })).toBeVisible({ timeout: 10_000 });
  // an invalid address is rejected by the core; its message comes back in Russian
  await page.getByTestId('tab-home').click();
  await page.getByTestId('add').click();
  await page.getByTestId('f-name').fill('X');
  await page.getByTestId('f-host').fill('-bad host');
  await page.getByTestId('f-user').fill('tester');
  await page.getByTestId('f-password').fill('p');
  await page.getByTestId('form-save').click();
  await expect(page.getByTestId('form-error')).toContainText('Некорректный адрес сервера');
});

test.describe('language: "auto" follows the browser language', () => {
  test.use({ locale: 'de-DE' });
  test('German browser -> German interface without any setting', async ({ page }) => {
    bridge = await startBridge();
    await page.goto(bridge.url);
    await expect(page.getByRole('heading', { name: 'Masterpasswort erstellen' })).toBeVisible();
    expect(await page.evaluate(() => document.documentElement.lang)).toBe('de');
  });
});
