import { test, expect } from '@playwright/test';
import { Bridge, startBridge } from '../fixtures';
import { acceptHostKey, addServer, connectFirst, createVault, PASS, removeOnServer, row } from '../helpers';

let bridge: Bridge;
let workDir = '';
test.afterEach(async () => {
  if (workDir) await removeOnServer(bridge.url, workDir).catch(() => undefined);
  workDir = '';
  await bridge?.stop();
});

test('проводник: создать, переименовать, копировать, вырезать, права, удалить', async ({ page }) => {
  bridge = await startBridge();
  await page.goto(bridge.url);
  await createVault(page);
  await addServer(page, { name: 'Ops', host: 'sshd-password', password: PASS });
  await connectFirst(page);
  await acceptHostKey(page);
  await expect(page.getByTestId('path-input')).toHaveValue('/home/tester');

  const prompt = async (text: string) => {
    await page.getByTestId('prompt-input').fill(text);
    await page.getByTestId('prompt-ok').click();
  };
  const work = `e2e-${Date.now()}`;
  workDir = `/home/tester/${work}`;

  // новая папка через кнопку, войти в неё
  await page.getByTestId('new-folder').click();
  await prompt(work);
  await expect(row(page, work)).toBeVisible();
  await row(page, work).dblclick();
  await expect(page.getByTestId('path-input')).toHaveValue(`/home/tester/${work}`);

  // новый файл, повтор имени — ошибка, ничего не затёрто
  await page.getByTestId('new-file').click();
  await prompt('заметка.txt');
  await expect(page.getByTestId('editor')).toBeVisible();
  await expect(row(page, 'заметка.txt')).toHaveCount(0); // создаётся только при первом сохранении
  await page.locator('.cm-content').click();
  await page.keyboard.type('первая строка');
  await page.keyboard.press('Control+s');
  await expect(page.getByTestId('ed-status')).toHaveText('Сохранено');
  await expect(row(page, 'заметка.txt')).toBeVisible();
  await page.getByTestId(`pane-close-f:/home/tester/${work}/заметка.txt`).click();
  await page.getByTestId('new-file').click();
  await prompt('заметка.txt');
  await expect(page.getByRole('alert').filter({ hasText: 'уже существует' })).toBeVisible();

  // новая папка через контекстное меню пустого места
  await page.getByTestId('explorer-list').click({ button: 'right', position: { x: 200, y: 300 } });
  await page.getByTestId('ctx-Новая папка').click();
  await prompt('sub');
  await expect(row(page, 'sub')).toBeVisible();

  // переименование по F2
  await row(page, 'заметка.txt').click();
  await page.keyboard.press('F2');
  await prompt('note 2.txt');
  await expect(row(page, 'note 2.txt')).toBeVisible();
  await expect(row(page, 'заметка.txt')).toHaveCount(0);

  // копировать / вставить в ту же папку: «(копия)»
  await row(page, 'note 2.txt').click();
  await page.keyboard.press('Control+c');
  await page.keyboard.press('Control+v');
  await expect(row(page, 'note 2 (копия).txt')).toBeVisible();
  await page.keyboard.press('Control+v');
  await expect(row(page, 'note 2 (копия 2).txt')).toBeVisible();

  // вырезать в папку sub через меню «Вставить в папку»
  await row(page, 'note 2 (копия).txt').click();
  await page.keyboard.press('Control+x');
  await row(page, 'sub').click({ button: 'right' });
  await page.getByTestId('ctx-Вставить в папку').click();
  await expect(row(page, 'note 2 (копия).txt')).toHaveCount(0);
  await row(page, 'sub').dblclick();
  await expect(row(page, 'note 2 (копия).txt')).toBeVisible();
  await page.getByTestId('up').click();
  await expect(row(page, 'sub')).toBeVisible();

  // права
  await row(page, 'note 2.txt').click({ button: 'right' });
  await page.getByTestId('ctx-Права доступа…').click();
  await page.getByTestId('chmod-octal').fill('600');
  await page.getByTestId('chmod-ok').click();
  await expect(row(page, 'note 2.txt').locator('.c-mode')).toHaveText('-rw-------');
  await page.screenshot({ path: 'screenshots/50-file-ops.png' });

  // удаление: отмена ничего не удаляет, подтверждение удаляет всё дерево
  await page.keyboard.press('Delete');
  await page.getByTestId('confirm-cancel').click();
  await expect(row(page, 'note 2.txt')).toBeVisible();
  await page.getByTestId('up').click();
  await row(page, work).click();
  await page.keyboard.press('Delete');
  await expect(page.getByTestId('confirm')).toContainText('папок');
  await page.getByTestId('confirm-ok').click();
  await expect(row(page, work)).toHaveCount(0);
});
