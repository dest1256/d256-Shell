import { test, expect, Page } from '@playwright/test';
import { Bridge, startBridge } from '../fixtures';
import { acceptHostKey, createVault } from '../helpers';

let bridge: Bridge;
test.afterEach(async () => bridge?.stop());

type XTerm = import('@xterm/xterm').Terminal;
const text = (page: Page) =>
  page.evaluate(() => {
    const t = (document.querySelector('[data-testid="terminal"]') as unknown as { __term: XTerm }).__term;
    let out = '';
    for (let i = 0; i < t.buffer.active.length; i++) out += (i && !t.buffer.active.getLine(i)?.isWrapped ? '\n' : '') + (t.buffer.active.getLine(i)?.translateToString(true) ?? '');
    return out;
  });

async function start(page: Page) {
  bridge = await startBridge();
  await page.goto(bridge.url);
  await createVault(page);
}

/** Add a server that logs in with a private key (no password at all). */
async function addKeyServer(page: Page, name: string, key: string, passphrase?: string) {
  await page.getByTestId('add').click();
  await page.getByTestId('f-name').fill(name);
  await page.getByTestId('f-host').fill('sshd-key');
  await page.getByTestId('f-user').fill('tester');
  await expect(page.getByTestId('key-box')).toBeHidden(); // скрыто, пока флажок не включён
  await page.getByTestId('f-usekey').check();
  await expect(page.getByTestId('key-box')).toBeVisible();
  await page.getByTestId('f-keypath').fill(`/testkeys/${key}`);
  if (passphrase) await page.getByTestId('f-keypass').fill(passphrase);
  await page.screenshot({ path: 'screenshots/61-server-key.png' });
  await page.getByTestId('form-save').click();
  await expect(page.getByTestId('server-item').filter({ hasText: name })).toBeVisible();
}

test('вход по ключу без пароля: SFTP и терминал, значок «ключ» в списке', async ({ page }) => {
  await start(page);
  await addKeyServer(page, 'Key1', 'id_ed25519');
  await expect(page.getByTestId('server-item').first()).toContainText('· ключ');
  await page.getByTestId('server-item').first().dblclick();
  await acceptHostKey(page);
  await expect(page.getByTestId('prompt-dialog')).toHaveCount(0); // ни пароля, ни парольной фразы
  await expect(page.getByTestId('path-input')).toHaveValue('/home/tester');
  await page.getByTestId('terminal').click();
  await expect.poll(() => text(page), { timeout: 20_000 }).toMatch(/\$ $/m);
  await page.keyboard.type('echo via-key-$((40+2))');
  await page.keyboard.press('Enter');
  await expect.poll(() => text(page)).toContain('via-key-42');
});

test('ключ с парольной фразой: сохранённая не спрашивается, несохранённая запрашивается и маскируется', async ({ page }) => {
  await start(page);
  await addKeyServer(page, 'Saved', 'id_enc', 'keypass');
  await addKeyServer(page, 'Ask', 'id_enc');

  // сохранённая фраза: вопросов нет
  await page.getByTestId('server-item').filter({ hasText: 'Saved' }).dblclick();
  await acceptHostKey(page);
  await expect(page.getByTestId('tab-s1')).toBeVisible();
  await expect(page.getByTestId('prompt-dialog')).toHaveCount(0);
  await page.getByTestId('tab-close-s1').click(); // one terminal on the page keeps the helpers simple
  await page.getByTestId('confirm-ok').click();
  await expect(page.getByTestId('tab-s1')).toHaveCount(0);
  await page.getByTestId('tab-home').click();

  // несохранённая: диалог «Парольная фраза ключа», поле скрыто; терминал спросит ещё раз (отдельный ssh)
  await page.getByTestId('server-item').filter({ hasText: 'Ask' }).dblclick();
  const dlg = page.getByTestId('prompt-dialog');
  await expect(dlg).toBeVisible();
  await expect(dlg).toContainText('Парольная фраза ключа');
  await expect(dlg.locator('input')).toHaveAttribute('type', 'password');
  await page.screenshot({ path: 'screenshots/62-passphrase.png' });
  await dlg.locator('input').fill('keypass');
  await page.getByTestId('prompt-ok').click();
  await expect(page.getByTestId('tab-s2')).toBeVisible();
  // терминал той же вкладки спрашивает фразу сам: отвечаем и попадаем в оболочку
  await expect(dlg).toBeVisible({ timeout: 20_000 });
  await dlg.locator('input').fill('keypass');
  await page.getByTestId('prompt-ok').click();
  await page.getByTestId('terminal').click();
  await expect.poll(() => text(page), { timeout: 20_000 }).toMatch(/\$ $/m);
});

test('понятные ошибки: файл .ppk, открытый ключ, чужой ключ; «Проверить подключение» с ключом', async ({ page }) => {
  await start(page);
  await page.getByTestId('add').click();
  await page.getByTestId('f-name').fill('Bad');
  await page.getByTestId('f-host').fill('sshd-key');
  await page.getByTestId('f-user').fill('tester');
  await page.getByTestId('f-usekey').check();
  let trusted = false;
  const test_ = async () => {
    await page.getByTestId('form-test').click();
    if (!trusted) {
      // the first connection asks about the server's host key (before any authentication)
      await page.getByTestId('hostkey-dialog').waitFor({ timeout: 10_000 }).then(() => page.getByTestId('hostkey-accept').click()).catch(() => undefined);
      trusted = true;
    }
  };
  await page.getByTestId('f-keypath').fill('/testkeys/id_putty.ppk');
  await test_();
  await expect(page.getByTestId('error-message')).toContainText('.ppk', { timeout: 15_000 });
  await page.getByTestId('error-ok').click();
  await page.getByTestId('f-keypath').fill('/testkeys/id_ed25519.pub');
  await test_();
  await expect(page.getByTestId('error-message')).toContainText('открытый', { timeout: 15_000 });
  await page.getByTestId('error-ok').click();
  await page.getByTestId('f-keypath').fill('/testkeys/id_other');
  await test_();
  await expect(page.getByTestId('error-message')).toContainText('не принял приватный ключ', { timeout: 15_000 });
  await page.getByTestId('error-ok').click();
  // правильный ключ: успех
  await page.getByTestId('f-keypath').fill('/testkeys/id_rsa');
  await test_();
  await expect(page.getByRole('alert').filter({ hasText: 'Подключение успешно' })).toBeVisible({ timeout: 15_000 });
});
