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

test('explorer: the .. row above all folders goes one level up', async ({ page }) => {
  await open(page);
  const rows = page.locator('[data-testid="explorer-row"]');
  await expect(rows.first()).toHaveAttribute('data-name', '..');
  await row(page, 'zone').dblclick();
  await expect(page.getByTestId('path-input')).toHaveValue('/home/tester/zone');
  await expect(rows.first()).toHaveAttribute('data-name', '..');
  await expect(page.getByTestId('explorer-status')).not.toContainText('Objects: 0');
  await rows.first().click();
  await page.keyboard.press('Delete'); // nothing to delete: no dialog
  await expect(page.getByTestId('confirm')).toHaveCount(0);
  await rows.first().dblclick();
  await expect(page.getByTestId('path-input')).toHaveValue('/home/tester');
  await page.getByTestId('path-input').fill('/');
  await page.getByTestId('path-input').press('Enter');
  await expect(page.getByTestId('path-input')).toHaveValue('/');
  await expect(page.locator('[data-testid="explorer-row"][data-name=".."]')).toHaveCount(0);
});

test('duplication: the second live shell on the right repeats the folder, venv and the running program', async ({ page }) => {
  await open(page);
  await page.getByTestId('terminal').click();
  // venv and the running program in the source terminal: tail -f keeps the terminal busy
  await page.keyboard.type("mkdir -p /tmp/vv/bin && printf 'export VIRTUAL_ENV=/tmp/vv\\n' > /tmp/vv/bin/activate && . /tmp/vv/bin/activate");
  await page.keyboard.press('Enter');
  await page.keyboard.type("printf 'UNIQUE-LINE-XYZ\\n' > /tmp/dupfile && cd /tmp && tail -f dupfile");
  await page.keyboard.press('Enter');
  await expect.poll(() => text(page, 'terminal')).toMatch(/\nUNIQUE-LINE-XYZ/);
  await page.waitForTimeout(500);

  // Ctrl+Shift+D: the dialog shows what will be repeated
  await page.keyboard.press('Control+Shift+d');
  const dlg = page.getByTestId('clone-dialog');
  await expect(dlg).toBeVisible();
  await expect(dlg).toContainText('folder /tmp');
  const lines = await page.getByTestId('clone-lines').inputValue();
  expect(lines.split('\n')).toEqual(["cd /tmp", ". /tmp/vv/bin/activate", "tail -f dupfile"]);
  await page.screenshot({ path: 'screenshots/58-clone-dialog.png' });
  await page.getByTestId('clone-ok').click();

  // a second panel appeared on the right; it has the earlier output and a new shell that replayed the commands
  await expect(page.getByTestId('side-col')).toBeVisible();
  const main = await page.getByTestId('terminal').boundingBox();
  const copyBox = await page.getByTestId('terminal-copy').boundingBox();
  expect(copyBox!.x).toBeGreaterThan(main!.x + main!.width - 2); // next to it, on the right
  await expect.poll(() => text(page, 'terminal-copy'), { timeout: 30_000 }).toMatch(/new shell[\s\S]*tail -f dupfile[\s\S]*UNIQUE-LINE-XYZ/);
  const copy = await text(page, 'terminal-copy');
  expect(count(copy, 'UNIQUE-LINE-XYZ')).toBeGreaterThanOrEqual(3); // earlier screen + the typed command? + new tail output
  await page.screenshot({ path: 'screenshots/59-terminal-split.png' });

  // copy in a venv: Ctrl+C in the copy returns the prompt, and the variable is still there
  await page.getByTestId('terminal-copy').click();
  await page.keyboard.press('Control+c');
  await page.keyboard.type('echo ENV=$VIRTUAL_ENV PWD=$PWD');
  await page.keyboard.press('Enter');
  await expect.poll(() => text(page, 'terminal-copy')).toContain('ENV=/tmp/vv PWD=/tmp');

  // the original is independent: stop tail in it and keep working, the copy did not change
  await page.getByTestId('terminal').click();
  await page.keyboard.press('Control+c');
  await page.keyboard.type('echo only-original');
  await page.keyboard.press('Enter');
  await expect.poll(() => text(page, 'terminal')).toContain('only-original');
  expect(await text(page, 'terminal-copy')).not.toContain('only-original');

  // closing the copy: confirmation, the column disappears
  await page.getByTestId('pane-close-dup:1').click();
  await page.getByTestId('confirm-ok').click();
  await expect(page.getByTestId('side-col')).toBeHidden();
});

test('duplication: view only and the remembered do not ask again choice', async ({ page }) => {
  await open(page);
  await page.getByTestId('terminal').click();
  await page.keyboard.type("cd /usr && echo marker-one");
  await page.keyboard.press('Enter');
  await expect.poll(() => text(page, 'terminal')).toContain('marker-one');
  await page.keyboard.press('Control+Shift+d');
  await page.getByTestId('clone-view').click();
  await expect.poll(() => text(page, 'terminal-copy'), { timeout: 30_000 }).toMatch(/new shell[\s\S]*Last login/);
  // no commands were run: the copy stayed in the home folder
  await page.getByTestId('terminal-copy').click();
  await page.keyboard.type('pwd');
  await page.keyboard.press('Enter');
  await expect.poll(() => text(page, 'terminal-copy')).toMatch(/\n\/home\/tester\n/);

  // do not ask again: the next copy repeats the folder at once, without a dialog
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

test('duplication: the terminal answers to shell queries (as with BusyBox ash) do not cancel running the commands', async ({ page }) => {
  await open(page);
  const term = page.getByTestId('terminal');
  await term.click();
  // the login shell asks the terminal for the cursor position (ESC[6n), xterm answers through onData
  await page.keyboard.type('cp ~/.profile /tmp/.profile.bak && cd /usr');
  await page.keyboard.press('Enter');
  await page.keyboard.type(String.raw`echo "printf '\033[6n'" >> ~/.profile`);
  await page.keyboard.press('Enter');
  try {
    await page.keyboard.press('Control+Shift+d');
    await expect(page.getByTestId('clone-dialog')).toBeVisible();
    await page.getByTestId('clone-ok').click();
    // the commands are typed in the copy anyway (not lost because the user typed something)
    await expect.poll(() => text(page, 'terminal-copy'), { timeout: 30_000 }).toMatch(/new shell[\s\S]*cd \/usr/);
  } finally {
    await term.click();
    await page.keyboard.type('cp /tmp/.profile.bak ~/.profile');
    await page.keyboard.press('Enter');
    await page.waitForTimeout(300);
  }
});

test('duplication: the earlier output is not shown on top for full-screen programs by default', async ({ page }) => {
  await open(page);
  await page.getByTestId('terminal').click();
  await page.keyboard.type('echo OLD-OUTPUT-MARK; cd /usr; sleep 600');
  await page.keyboard.press('Enter');
  await page.waitForTimeout(500);
  await page.keyboard.press('Control+Shift+d');
  await expect(page.getByTestId('clone-prefill')).toBeChecked(); // sleep: an ordinary program, we show the output
  await page.getByTestId('clone-cancel').click();
  // a full-screen program (imitated by its name): top
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

test('duplication after su: the copy logs in as the same user and repeats the environment', async ({ page }) => {
  bridge = await startBridge();
  await page.goto(bridge.url);
  await createVault(page);
  await addServer(page, { name: 'Root', host: 'sshd-password', password: 'rootpass', user: 'root' });
  await connectFirst(page);
  await acceptHostKey(page);
  await expect(page.getByTestId('tab-s1')).toBeVisible();
  const term = page.getByTestId('terminal');
  await term.click();
  // as su - comfyui, cd ~/ComfyUI && source .venv/bin/activate
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
  await expect(page.getByTestId('clone-dialog')).toContainText('login: su - otheruser');
  const lines = (await page.getByTestId('clone-lines').inputValue()).split('\n');
  expect(lines[0]).toBe('su - otheruser');
  expect(lines[lines.length - 1]).toBe('source vv/bin/activate');
  await page.screenshot({ path: 'screenshots/60-clone-su.png' });
  await page.getByTestId('clone-ok').click();

  // in the copy: login as otheruser, then folder and environment
  await page.getByTestId('terminal-copy').click();
  await expect.poll(() => text(page, 'terminal-copy'), { timeout: 40_000 }).toMatch(/new shell[\s\S]*su - otheruser[\s\S]*source vv\/bin\/activate/);
  await page.keyboard.type('echo WHO=$(id -un) VENV=$VIRTUAL_ENV');
  await page.keyboard.press('Enter');
  await expect.poll(() => text(page, 'terminal-copy')).toContain('WHO=otheruser VENV=/home/otheruser/vv');
});
