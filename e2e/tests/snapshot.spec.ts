import { test, expect, Page } from '@playwright/test';
import { Bridge, startBridge } from '../fixtures';
import { acceptHostKey, addServer, connectFirst, createVault, PASS, row } from '../helpers';

let bridge: Bridge;
test.afterEach(async () => bridge?.stop());

type XTerm = import('@xterm/xterm').Terminal;

const text = (page: Page, testid: string) =>
  page.evaluate((id) => {
    const t = (document.querySelector(`[data-testid="${id}"]`) as unknown as { __term: XTerm }).__term;
    const b = t.buffer.active;
    // soft-wrapped lines (a narrow panel) are joined back into one logical line
    let out = '';
    for (let i = 0; i < b.length; i++) {
      const l = b.getLine(i);
      out += (i > 0 && !l?.isWrapped ? '\n' : '') + (l?.translateToString(true) ?? '');
    }
    return out;
  }, testid);

async function open(page: Page) {
  bridge = await startBridge();
  await page.goto(bridge.url);
  await createVault(page);
  await addServer(page, { name: 'Dup', host: 'sshd-password', password: PASS });
  await connectFirst(page);
  await acceptHostKey(page);
  await expect(page.getByTestId('path-input')).toHaveValue('/home/tester');
}

const count = (s: string, needle: string) => s.split(needle).length - 1;

test('проводник: строка «..» над всеми папками выходит на уровень вверх', async ({ page }) => {
  await open(page);
  const rows = page.locator('[data-testid="explorer-row"]');
  await expect(rows.first()).toHaveAttribute('data-name', '..');
  await row(page, 'zone').dblclick();
  await expect(page.getByTestId('path-input')).toHaveValue('/home/tester/zone');
  await expect(rows.first()).toHaveAttribute('data-name', '..');
  await expect(page.getByTestId('explorer-status')).not.toContainText('Объектов: 0');
  await rows.first().click();
  await page.keyboard.press('Delete'); // удалять нечего: диалога нет
  await expect(page.getByTestId('confirm')).toHaveCount(0);
  await rows.first().dblclick();
  await expect(page.getByTestId('path-input')).toHaveValue('/home/tester');
  await page.getByTestId('path-input').fill('/');
  await page.getByTestId('path-input').press('Enter');
  await expect(page.getByTestId('path-input')).toHaveValue('/');
  await expect(page.locator('[data-testid="explorer-row"][data-name=".."]')).toHaveCount(0);
});

test('дублирование: вторая живая оболочка справа повторяет папку, venv и запущенную программу', async ({ page }) => {
  await open(page);
  await page.getByTestId('terminal').click();
  // «venv» и «рабочая программа» в исходном терминале: tail -f держит терминал занятым
  await page.keyboard.type("mkdir -p /tmp/vv/bin && printf 'export VIRTUAL_ENV=/tmp/vv\\n' > /tmp/vv/bin/activate && . /tmp/vv/bin/activate");
  await page.keyboard.press('Enter');
  await page.keyboard.type("printf 'UNIQUE-LINE-XYZ\\n' > /tmp/dupfile && cd /tmp && tail -f dupfile");
  await page.keyboard.press('Enter');
  await expect.poll(() => text(page, 'terminal')).toMatch(/\nUNIQUE-LINE-XYZ/);
  await page.waitForTimeout(500);

  // Ctrl+Shift+D: диалог показывает, что будет повторено
  await page.keyboard.press('Control+Shift+d');
  const dlg = page.getByTestId('clone-dialog');
  await expect(dlg).toBeVisible();
  await expect(dlg).toContainText('папка /tmp');
  const lines = await page.getByTestId('clone-lines').inputValue();
  expect(lines.split('\n')).toEqual(["cd /tmp", ". /tmp/vv/bin/activate", "tail -f dupfile"]);
  await page.screenshot({ path: 'screenshots/58-clone-dialog.png' });
  await page.getByTestId('clone-ok').click();

  // справа появилась вторая панель; в ней прежний вывод и новая оболочка, которая повторила команды
  await expect(page.getByTestId('side-col')).toBeVisible();
  const main = await page.getByTestId('terminal').boundingBox();
  const copyBox = await page.getByTestId('terminal-copy').boundingBox();
  expect(copyBox!.x).toBeGreaterThan(main!.x + main!.width - 2); // рядом, справа
  await expect.poll(() => text(page, 'terminal-copy'), { timeout: 30_000 }).toMatch(/новая оболочка[\s\S]*tail -f dupfile[\s\S]*UNIQUE-LINE-XYZ/);
  const copy = await text(page, 'terminal-copy');
  expect(count(copy, 'UNIQUE-LINE-XYZ')).toBeGreaterThanOrEqual(3); // прежний экран + набранная команда? + новый вывод tail
  await page.screenshot({ path: 'screenshots/59-terminal-split.png' });

  // копия «в среде venv»: Ctrl+C в копии возвращает приглашение, и переменная на месте
  await page.getByTestId('terminal-copy').click();
  await page.keyboard.press('Control+c');
  await page.keyboard.type('echo ENV=$VIRTUAL_ENV PWD=$PWD');
  await page.keyboard.press('Enter');
  await expect.poll(() => text(page, 'terminal-copy')).toContain('ENV=/tmp/vv PWD=/tmp');

  // оригинал независим: прерываем в нём tail и работаем дальше, копия не изменилась
  await page.getByTestId('terminal').click();
  await page.keyboard.press('Control+c');
  await page.keyboard.type('echo only-original');
  await page.keyboard.press('Enter');
  await expect.poll(() => text(page, 'terminal')).toContain('only-original');
  expect(await text(page, 'terminal-copy')).not.toContain('only-original');

  // закрытие копии: подтверждение, колонка исчезает
  await page.getByTestId('pane-close-dup:1').click();
  await page.getByTestId('confirm-ok').click();
  await expect(page.getByTestId('side-col')).toBeHidden();
});

test('дублирование: «только вывод» и запомненный выбор «не спрашивать»', async ({ page }) => {
  await open(page);
  await page.getByTestId('terminal').click();
  await page.keyboard.type("cd /usr && echo marker-one");
  await page.keyboard.press('Enter');
  await expect.poll(() => text(page, 'terminal')).toContain('marker-one');
  await page.keyboard.press('Control+Shift+d');
  await page.getByTestId('clone-view').click();
  await expect.poll(() => text(page, 'terminal-copy'), { timeout: 30_000 }).toMatch(/новая оболочка[\s\S]*Last login/);
  // команд не выполнялось: копия осталась в домашней папке
  await page.getByTestId('terminal-copy').click();
  await page.keyboard.type('pwd');
  await page.keyboard.press('Enter');
  await expect.poll(() => text(page, 'terminal-copy')).toMatch(/\n\/home\/tester\n/);

  // «не спрашивать»: следующая копия повторяет папку сразу, без диалога
  await page.getByTestId('terminal').click();
  await page.keyboard.press('Control+Shift+d');
  await page.getByTestId('clone-remember').check();
  await page.getByTestId('clone-ok').click();
  await page.getByTestId('terminal').click();
  await page.keyboard.press('Control+Shift+d');
  await expect(page.getByTestId('clone-dialog')).toHaveCount(0);
  await expect(page.getByTestId('terminal-copy-3')).toBeAttached({ timeout: 15_000 }); // the context is read first, then the copy opens
  await expect.poll(() => text(page, 'terminal-copy-3'), { timeout: 30_000 }).toMatch(/cd \/usr/);
  await expect(page.locator('[data-testid^="pane-dup"]')).toHaveCount(3);
});

test('дублирование: ответы терминала на запросы оболочки (как у BusyBox ash) не отменяют запуск команд', async ({ page }) => {
  await open(page);
  const term = page.getByTestId('terminal');
  await term.click();
  // логин-оболочка просит у терминала положение курсора (ESC[6n), xterm отвечает через onData
  await page.keyboard.type('cp ~/.profile /tmp/.profile.bak && cd /usr');
  await page.keyboard.press('Enter');
  await page.keyboard.type(String.raw`echo "printf '\033[6n'" >> ~/.profile`);
  await page.keyboard.press('Enter');
  try {
    await page.keyboard.press('Control+Shift+d');
    await expect(page.getByTestId('clone-dialog')).toBeVisible();
    await page.getByTestId('clone-ok').click();
    // команды всё равно набраны в копии (а не потеряны из-за «пользователь что-то ввёл»)
    await expect.poll(() => text(page, 'terminal-copy'), { timeout: 30_000 }).toMatch(/новая оболочка[\s\S]*cd \/usr/);
  } finally {
    await term.click();
    await page.keyboard.type('cp /tmp/.profile.bak ~/.profile');
    await page.keyboard.press('Enter');
    await page.waitForTimeout(300);
  }
});

test('дублирование: прежний вывод сверху не показывается для полноэкранных программ по умолчанию', async ({ page }) => {
  await open(page);
  await page.getByTestId('terminal').click();
  await page.keyboard.type('echo OLD-OUTPUT-MARK; cd /usr; sleep 600');
  await page.keyboard.press('Enter');
  await page.waitForTimeout(500);
  await page.keyboard.press('Control+Shift+d');
  await expect(page.getByTestId('clone-prefill')).toBeChecked(); // sleep: обычная программа, вывод показываем
  await page.getByTestId('clone-cancel').click();
  // полноэкранная программа (имитируем именем): top
  await page.getByTestId('terminal').click();
  await page.keyboard.press('Control+c');
  await page.keyboard.type('top -b -d 5 > /dev/null');
  await page.keyboard.press('Enter');
  await page.waitForTimeout(500);
  await page.keyboard.press('Control+Shift+d');
  await expect(page.getByTestId('clone-prefill')).not.toBeChecked();
  await page.getByTestId('clone-ok').click();
  await expect(page.getByTestId('terminal-copy')).toBeAttached({ timeout: 15_000 });
  await page.waitForTimeout(1500);
  expect(await text(page, 'terminal-copy')).not.toContain('OLD-OUTPUT-MARK');
});

test('дублирование после su: копия входит под тем же пользователем и повторяет окружение', async ({ page }) => {
  bridge = await startBridge();
  await page.goto(bridge.url);
  await createVault(page);
  await addServer(page, { name: 'Root', host: 'sshd-password', password: 'rootpass', user: 'root' });
  await connectFirst(page);
  await acceptHostKey(page);
  await expect(page.getByTestId('tab-s1')).toBeVisible();
  const term = page.getByTestId('terminal');
  await term.click();
  // как «su - comfyui, cd ~/ComfyUI && source .venv/bin/activate»
  await page.keyboard.type('su - otheruser');
  await page.keyboard.press('Enter');
  await expect.poll(() => text(page, 'terminal')).toContain('otheruser@');
  await page.keyboard.type("mkdir -p ~/vv/bin && echo 'export VIRTUAL_ENV=$HOME/vv' > ~/vv/bin/activate");
  await page.keyboard.press('Enter');
  await page.keyboard.type('cd ~ && source vv/bin/activate');
  await page.keyboard.press('Enter');
  await page.waitForTimeout(600);

  await page.keyboard.press('Control+Shift+d');
  await expect(page.getByTestId('clone-dialog')).toBeVisible();
  await expect(page.getByTestId('clone-dialog')).toContainText('вход: su - otheruser');
  const lines = (await page.getByTestId('clone-lines').inputValue()).split('\n');
  expect(lines[0]).toBe('su - otheruser');
  expect(lines[lines.length - 1]).toBe('source vv/bin/activate');
  await page.screenshot({ path: 'screenshots/60-clone-su.png' });
  await page.getByTestId('clone-ok').click();

  // в копии: вход под otheruser, затем папка и окружение
  await page.getByTestId('terminal-copy').click();
  await expect.poll(() => text(page, 'terminal-copy'), { timeout: 40_000 }).toMatch(/новая оболочка[\s\S]*su - otheruser[\s\S]*source vv\/bin\/activate/);
  await page.keyboard.type('echo WHO=$(id -un) VENV=$VIRTUAL_ENV');
  await page.keyboard.press('Enter');
  await expect.poll(() => text(page, 'terminal-copy')).toContain('WHO=otheruser VENV=/home/otheruser/vv');
});
