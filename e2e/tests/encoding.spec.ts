import { test, expect, Page } from '@playwright/test';
import { Bridge, startBridge } from '../fixtures';
import { acceptHostKey, addServer, connectFirst, createVault, PASS, removeOnServer, row, serverReadEnc, serverWriteEnc } from '../helpers';

let bridge: Bridge;
let work = '';
test.afterEach(async () => {
  if (work) await removeOnServer(bridge.url, work).catch(() => undefined);
  work = '';
  await bridge?.stop();
});

const TEXT = '# Настройки сервера\nпароль=секрет\nадрес=example.org\n';
const editorText = (page: Page) => page.locator('.cm-content').innerText();

async function setup(page: Page) {
  bridge = await startBridge();
  await page.goto(bridge.url);
  await createVault(page);
  await addServer(page, { name: 'Enc', host: 'sshd-password', password: PASS });
  await connectFirst(page);
  await acceptHostKey(page);
  await expect(page.getByTestId('path-input')).toHaveValue('/home/tester');
  work = `/home/tester/e2e-enc-${Date.now()}`;
  await page.getByTestId('new-folder').click();
  await page.getByTestId('prompt-input').fill(work.split('/').pop()!);
  await page.getByTestId('prompt-ok').click();
  await row(page, work.split('/').pop()!).dblclick();
  await expect(page.getByTestId('path-input')).toHaveValue(work);
}

test('кириллический файл в Windows-1251 открывается без вопросов и сохраняется в той же кодировке', async ({ page }) => {
  await setup(page);
  await serverWriteEnc(bridge.url, `${work}/.env`, TEXT, 'windows-1251');
  await page.getByTestId('refresh').click();
  await row(page, '.env').dblclick();
  await expect(page.getByTestId('editor')).toBeVisible(); // раньше: «Файл нельзя открыть в редакторе»
  await expect(page.getByTestId('confirm')).toHaveCount(0);
  await expect.poll(() => editorText(page)).toContain('пароль=секрет');
  await expect(page.getByTestId('ed-enc')).toHaveText('windows-1251');
  await expect(page.getByTestId('ed-status')).toContainText('определена автоматически');

  await page.locator('.cm-content').click();
  await page.keyboard.press('Control+End');
  await page.keyboard.type('порт=22');
  await page.keyboard.press('Control+s');
  await expect(page.getByTestId('ed-status')).toHaveText('Сохранено');
  expect(await serverReadEnc(bridge.url, `${work}/.env`, 'windows-1251')).toBe(TEXT + 'порт=22');
  await page.screenshot({ path: 'screenshots/63-encoding.png' });
});

test('выбор кодировки: перечитать неверно угаданную, конвертировать при сохранении, символ вне кодировки не теряется', async ({ page }) => {
  await setup(page);
  await serverWriteEnc(bridge.url, `${work}/koi.txt`, TEXT, 'KOI8-R');
  await page.getByTestId('refresh').click();
  await row(page, 'koi.txt').dblclick();
  await expect(page.getByTestId('editor')).toBeVisible();
  // автоопределение могло ошибиться: вручную выбираем KOI8-R
  await page.getByTestId('ed-enc').click();
  await page.getByTestId('enc-select').selectOption('KOI8-R');
  await page.getByTestId('enc-reread').click();
  await expect.poll(() => editorText(page)).toContain('пароль=секрет');
  await expect(page.getByTestId('ed-enc')).toHaveText('KOI8-R');

  // символ, которого нет в KOI8-R: сохранение отказывает и ничего не пишет
  await page.locator('.cm-content').click();
  await page.keyboard.press('Control+End');
  await page.keyboard.type('цена=5ß');
  await page.keyboard.press('Control+s');
  await expect(page.getByRole('alert').filter({ hasText: 'нельзя записать в кодировке' })).toBeVisible();
  expect(await serverReadEnc(bridge.url, `${work}/koi.txt`, 'KOI8-R')).toBe(TEXT);

  // конвертация в UTF-8 через «Сохранять в этой»
  await page.getByTestId('ed-enc').click();
  await page.getByTestId('enc-select').selectOption('UTF-8');
  await page.getByTestId('enc-save-as').click();
  await expect(page.locator('.cm-content')).toBeFocused(); // focus returns to the editor after the dialog
  await page.keyboard.press('Control+s');
  await expect(page.getByTestId('ed-status')).toHaveText('Сохранено');
  expect(await serverReadEnc(bridge.url, `${work}/koi.txt`, 'UTF-8')).toBe(TEXT + 'цена=5ß');
  await page.getByTestId('ed-enc').click();
  await expect(page.getByTestId('enc-dialog')).toBeVisible();
  await page.screenshot({ path: 'screenshots/64-encoding-dialog.png' });
});
