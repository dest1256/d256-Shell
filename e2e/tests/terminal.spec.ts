import { test, expect, Page } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { Bridge, startBridge } from '../fixtures';
import { acceptHostKey, addServer, connectFirst, createVault, PASS, totp } from '../helpers';

let bridge: Bridge;
test.afterEach(async () => bridge?.stop());

type XTerm = import('@xterm/xterm').Terminal;

/** Whole terminal buffer (scrollback included) as text. */
const buffer = (page: Page) =>
  page.evaluate(() => {
    const t = (document.querySelector('[data-testid="terminal"]') as unknown as { __term: XTerm }).__term;
    const b = t.buffer.active;
    const out: string[] = [];
    for (let i = 0; i < b.length; i++) out.push(b.getLine(i)?.translateToString(true) ?? '');
    return out.join('\n');
  });

const escapeRe = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

async function waitFor(page: Page, text: string | RegExp, timeout = 30_000) {
  const re = text instanceof RegExp ? text : new RegExp(escapeRe(text));
  await expect.poll(() => buffer(page), { timeout, message: `terminal should contain ${re}` }).toMatch(re);
}

async function openTerminal(page: Page, host = 'sshd-password', password = PASS) {
  bridge = await startBridge();
  await page.goto(bridge.url);
  await createVault(page);
  await addServer(page, { name: 'T', host, password });
  await connectFirst(page);
  await acceptHostKey(page);
  await expect(page.getByTestId('tab-s1')).toBeVisible();
  await waitFor(page, /\$ $/m);
  await page.getByTestId('terminal').click();
}

/**
 * Type ASCII with real key events. Non-ASCII text must NOT go through `keyboard.type()`: for characters
 * missing from the US layout Playwright falls back to `insertText`, which xterm handles asynchronously
 * and which reorders input in automation (a test artefact: real keyboards send key events). Use `paste()`.
 */
const type = async (page: Page, s: string) => {
  await page.keyboard.type(s);
  await page.keyboard.press('Enter');
};

const paste = (page: Page, text: string) =>
  page.evaluate((t) => {
    const dt = new DataTransfer();
    dt.setData('text', t);
    document.querySelector('.term-host')!.dispatchEvent(new ClipboardEvent('paste', { clipboardData: dt, bubbles: true, cancelable: true }));
  }, text);

test('вход без ввода пароля, команда, вывод, индикатор состояния', async ({ page }) => {
  await openTerminal(page);
  await expect(page.getByTestId('tab-state-s1')).toHaveAttribute('data-state', 'ok');
  await type(page, 'echo marker-$((1000+234))');
  await waitFor(page, /^marker-1234$/m);
  const text = await buffer(page);
  expect(text.toLowerCase()).not.toContain('password');
  await expect(page.getByTestId('prompt-dialog')).toHaveCount(0);
  await page.screenshot({ path: 'screenshots/30-terminal.png' });
});

test('Ctrl+C уходит на сервер как ^C', async ({ page }) => {
  await openTerminal(page);
  await type(page, 'sleep 30');
  await page.waitForTimeout(500);
  await page.keyboard.press('Control+c');
  await waitFor(page, /\^C/);
  await type(page, 'echo alive-$((2*21))');
  await waitFor(page, /^alive-42$/m);
});

test('завершение: код выхода, Enter — переподключение, Esc — закрытие вкладки', async ({ page }) => {
  await openTerminal(page);
  await type(page, 'exit 3');
  await waitFor(page, /Сеанс завершён, код 3/);
  await expect(page.getByTestId('tab-state-s1')).toHaveAttribute('data-state', 'off');
  await page.screenshot({ path: 'screenshots/31-session-ended.png' });

  await page.keyboard.press('Enter'); // reconnect in the same tab
  await expect(page.getByTestId('tab-state-s1')).toHaveAttribute('data-state', 'ok');
  // the first session's last prompt line carries "exit 3", so a line that ends with "$ " is the new prompt
  await expect.poll(async () => (await buffer(page)).split('\n').filter((l) => /\$ $/.test(l)).length, { timeout: 30_000 }).toBeGreaterThan(0);
  await type(page, 'echo again-$((10+11))');
  await waitFor(page, /^again-21$/m);

  await type(page, 'exit');
  await waitFor(page, /Сеанс завершён, код 0/);
  await page.keyboard.press('Escape'); // closes the tab (no confirmation for an ended session)
  await expect(page.getByTestId('tab-s1')).toHaveCount(0);
});

test('вставка нескольких строк предупреждает, одна строка — нет', async ({ page }) => {
  await openTerminal(page);
  const p = (text: string) => paste(page, text);

  await p('echo one\necho two\necho three\n');
  await expect(page.getByTestId('paste-confirm')).toBeVisible();
  await expect(page.getByTestId('paste-confirm')).toContainText('Вставить 4 строк');
  await page.screenshot({ path: 'screenshots/32-paste-warning.png' });
  await page.getByTestId('paste-cancel').click();
  await page.waitForTimeout(400);
  expect(await buffer(page)).not.toContain('echo one');

  await p('echo one-$((3+4))\necho two-$((3+5))\n');
  await page.getByTestId('paste-ok').click();
  // bracketed paste: the shell inserts the text but does NOT execute it until the user presses Enter
  await waitFor(page, /echo two-\$\(\(3\+5\)\)/);
  await page.waitForTimeout(600);
  expect(await buffer(page)).not.toMatch(/^one-7$/m);
  await page.keyboard.press('Enter');
  await waitFor(page, /^one-7$/m);
  await waitFor(page, /^two-8$/m);

  // a single line is pasted without asking
  await p('echo single-$((6*7))');
  await expect(page.getByTestId('paste-confirm')).toHaveCount(0);
  await page.keyboard.press('Enter');
  await waitFor(page, /^single-42$/m);

  // a very large single line is also confirmed
  await p('#' + 'x'.repeat(6000));
  await expect(page.getByTestId('paste-warning')).toBeVisible();
  await page.getByTestId('paste-cancel').click();
});

test('размер терминала доходит до сервера', async ({ page }) => {
  await openTerminal(page);
  await page.setViewportSize({ width: 1000, height: 620 });
  await page.waitForTimeout(600);
  const dims = await page.evaluate(() => {
    const t = (document.querySelector('[data-testid="terminal"]') as unknown as { __term: XTerm }).__term;
    return { cols: t.cols, rows: t.rows };
  });
  await type(page, 'stty size');
  await waitFor(page, new RegExp(`^${dims.rows} ${dims.cols}$`, 'm'));
});

test('UTF-8: кириллица, символы, эмодзи; поток без потерь', async ({ page }) => {
  await openTerminal(page);
  // Cyrillic input (UTF-16 -> UTF-8) reaches the server and comes back unchanged
  await paste(page, 'echo привет-мир-$((6*7))');
  await page.keyboard.press('Enter');
  await waitFor(page, /^привет-мир-42$/m);
  // the emoji are produced by the server from UTF-8 bytes: typing a surrogate pair through the
  // browser automation is unreliable and is not what is being tested here
  const sample = "$(printf '\\xd1\\x91\\xd0\\xb6\\xd0\\xb8\\xd0\\xba-\\xe2\\x9c\\x93-\\xf0\\x9f\\x99\\x82')";
  await type(page, `printf '%s\\n' "${sample}"`);
  await waitFor(page, /^ёжик-✓-🙂$/m);
  await type(page, `yes "${sample}" | head -n 30000; echo END-$((6*7))`);
  await waitFor(page, /^END-42$/m, 60_000);
  const text = await buffer(page);
  expect(text).not.toContain('�');
  expect(text.split('\n').filter((l) => l === 'ёжик-✓-🙂').length).toBeGreaterThan(9000); // scrollback is 10 000 lines
});

test('поиск по выводу (Ctrl+Shift+F)', async ({ page }) => {
  await openTerminal(page);
  await type(page, 'echo needle-$((5+5))');
  await waitFor(page, /^needle-10$/m);
  await page.keyboard.press('Control+Shift+F');
  await expect(page.getByTestId('term-search')).toBeVisible();
  await page.getByTestId('term-search-input').fill('needle-10');
  await page.getByTestId('term-search-input').press('Enter');
  await page.getByTestId('term-search-input').press('Escape');
  await expect(page.getByTestId('term-search')).toBeHidden();
});

test('Ctrl+B/W внутри терминала уходят на сервер, Ctrl+Shift+B — приложению', async ({ page }) => {
  await openTerminal(page);
  await page.keyboard.press('Control+b'); // tmux/readline key: must not toggle the sidebar
  await expect(page.getByTestId('sidebar')).toBeHidden(); // hidden automatically in an SSH tab
  await page.keyboard.press('Control+Shift+b');
  await expect(page.getByTestId('sidebar')).toBeVisible();
  await page.keyboard.press('Control+Shift+b');
  await expect(page.getByTestId('sidebar')).toBeHidden();
  await page.getByTestId('terminal').click();
  await page.keyboard.press('Control+w'); // readline word-rubout: must not close the tab
  await expect(page.getByTestId('tab-s1')).toBeVisible();
});

test('настройки: схема и размер шрифта применяются к открытому терминалу', async ({ page }) => {
  await openTerminal(page);
  await page.getByTestId('settings').click();
  await expect(page.getByTestId('ssh-info')).toContainText('OpenSSH_');
  await expect(page.getByTestId('ssh-info')).toContainText('askpass');
  await page.screenshot({ path: 'screenshots/33-settings.png' });
  await page.getByTestId('set-scheme').selectOption('onehalfdark');
  await page.getByTestId('set-font-size').fill('18');
  await page.getByTestId('settings-save').click();
  await expect(page.getByTestId('settings-dialog')).toHaveCount(0);
  const o = await page.evaluate(() => {
    const t = (document.querySelector('[data-testid="terminal"]') as unknown as { __term: XTerm }).__term;
    return { size: t.options.fontSize, bg: t.options.theme?.background };
  });
  expect(o).toEqual({ size: 18, bg: '#282C34' });
  const saved = JSON.parse(readFileSync(join(bridge.dataDir, 'settings.json'), 'utf8'));
  expect(saved.scheme).toBe('onehalfdark');
  expect(saved.font_size).toBe(18);
});

test('сервер без SFTP: терминал работает, панель файлов объясняет причину', async ({ page }) => {
  bridge = await startBridge();
  await page.goto(bridge.url);
  await createVault(page);
  await addServer(page, { name: 'NoSftp', host: 'sshd-nosftp', password: PASS });
  await connectFirst(page);
  await acceptHostKey(page);
  await expect(page.getByTestId('tab-s1')).toBeVisible();
  await expect(page.getByTestId('sftp-unavailable')).toContainText('SFTP');
  await expect(page.getByTestId('error-dialog')).toHaveCount(0);
  await waitFor(page, /\$ $/m);
  await page.getByTestId('terminal').click();
  await type(page, 'echo shell-ok-$((40+2))');
  await waitFor(page, /^shell-ok-42$/m);
  await page.screenshot({ path: 'screenshots/34-no-sftp.png' });
});

test('2FA: код для терминала запрашивается у пользователя', async ({ page }) => {
  bridge = await startBridge();
  await page.goto(bridge.url);
  await createVault(page);
  await addServer(page, { name: '2FA', host: 'sshd-2fa', password: PASS });
  await connectFirst(page);
  await acceptHostKey(page);
  await page.getByTestId('prompt-input').fill(totp()); // SFTP login
  await page.getByTestId('prompt-ok').click();
  await expect(page.getByTestId('tab-s1')).toBeVisible();
  // the terminal's own ssh login asks for the code again
  await expect(page.getByTestId('prompt-dialog')).toBeVisible({ timeout: 20_000 });
  await expect(page.getByTestId('prompt-text')).toContainText(/verification code/i);
  await page.getByTestId('prompt-input').fill(totp());
  await page.getByTestId('prompt-ok').click();
  await waitFor(page, /\$ $/m);
});

test('колесо мыши в полноэкранной программе с мышью (btop) шлёт стрелки', async ({ page }) => {
  await openTerminal(page);
  // как btop: альтернативный экран + отслеживание мыши; cat -v показывает, что реально пришло
  await type(page, String.raw`printf '\033[?1049h\033[?1000h\033[?1006h'; cat -v`);
  await page.waitForTimeout(500);
  const box = (await page.getByTestId('terminal').boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.wheel(0, -120);
  await waitFor(page, '^[[A^[[A^[[A');
  await page.mouse.wheel(0, 120);
  await waitFor(page, '^[[B^[[B^[[B');
});

test('окно вставки: текст можно править, вставляется отредактированное', async ({ page }) => {
  await openTerminal(page);
  await paste(page, 'echo keep-1\necho DANGEROUS-LINE\necho keep-2\n');
  const ta = page.getByTestId('paste-text');
  await expect(ta).toBeVisible();
  await expect(ta).toBeFocused();
  await expect(ta).toHaveValue('echo keep-1\necho DANGEROUS-LINE\necho keep-2\n');
  // правка: заголовок и счётчик следуют за текстом, опасная строка убрана, одна исправлена
  await ta.fill('echo keep-1\necho fixed-$((6*7))\n');
  await expect(page.getByTestId('paste-confirm')).toContainText('Вставить 3 строк');
  await page.screenshot({ path: 'screenshots/68-paste-edit.png' });
  await page.getByTestId('paste-ok').click();
  await page.keyboard.press('Enter');
  await waitFor(page, /^fixed-42$/m);
  expect(await buffer(page)).not.toMatch(/^DANGEROUS-LINE$/m);

  // Ctrl+Enter вставляет прямо из поля
  await paste(page, 'echo ctrl-enter-1\necho ctrl-enter-2\n');
  await page.getByTestId('paste-text').press('Control+Enter');
  await expect(page.getByTestId('paste-confirm')).toHaveCount(0);
  await page.keyboard.press('Enter');
  await waitFor(page, /^ctrl-enter-2$/m);

  // пустой текст после правки = отмена, ничего не вставляется
  await paste(page, 'echo never-1\necho never-2\n');
  await page.getByTestId('paste-text').fill('');
  await page.getByTestId('paste-ok').click();
  await page.waitForTimeout(400);
  expect(await buffer(page)).not.toContain('echo never-1');
});
