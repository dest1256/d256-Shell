import { test, expect, Page } from '@playwright/test';
import { Bridge, startBridge } from '../fixtures';
import { acceptHostKey, addServer, connectFirst, createVault, PASS, rpc } from '../helpers';

type XTerm = import('@xterm/xterm').Terminal;

let bridge: Bridge;
test.afterEach(async () => bridge?.stop());

/** The text on the screen and in the scrollback of every local terminal. */
const screen = (page: Page) =>
  page.evaluate(() =>
    [...document.querySelectorAll('[data-testid="local-terminal"]')]
      .map((e) => {
        const t = (e as unknown as { __term: XTerm }).__term.buffer.active;
        let s = '';
        for (let i = 0; i < t.length; i++) s += t.getLine(i)?.translateToString(true) + '\n';
        return s;
      })
      .join('|'),
  );

async function start(page: Page) {
  bridge = await startBridge();
  await page.goto(bridge.url);
  await createVault(page);
}

async function runScript(page: Page, text: string) {
  await rpc(bridge.url, 'script_create', { name: 'prog.sh', text });
  await page.getByTestId('rail-scripts').click();
  await page.getByTestId('script-item').filter({ hasText: 'prog.sh' }).locator('.srv-name').dblclick();
  await expect(page.getByTestId('tab-local1')).toBeVisible();
}

test('restart a script from the tab menu: the new version of the file runs in the same tab', async ({ page }) => {
  await start(page);
  await runScript(page, 'echo first-version-$$; sleep 120\n');
  await expect.poll(() => screen(page), { timeout: 20_000 }).toContain('first-version-');
  // the program was edited meanwhile
  await rpc(bridge.url, 'script_write', { id: 'own:prog.sh', text: 'echo second-version-$$; sleep 120\n', eol: 'lf', bom: false, encoding: 'utf-8' });
  await page.getByTestId('tab-local1').click({ button: 'right' });
  await expect(page.getByTestId('ctx-menu')).toBeVisible();
  await page.screenshot({ path: 'screenshots/88-tab-menu.png' });
  await page.getByTestId('tabmenu-restart').click();
  // it is running: asks first
  await expect(page.getByTestId('restart-dialog')).toBeVisible();
  await page.getByTestId('restart-cancel').click();
  await expect(page.getByTestId('restart-dialog')).toHaveCount(0);
  expect(await screen(page)).toContain('first-version-'); // cancelled: nothing happened
  await page.getByTestId('tab-local1').click({ button: 'right' });
  await page.getByTestId('tabmenu-restart').click();
  await page.getByTestId('restart-ok').click();
  await expect.poll(() => screen(page), { timeout: 20_000 }).toContain('second-version-');
  expect(await screen(page)).not.toContain('first-version-'); // the screen was cleared
  await expect(page.locator('[data-testid^="tab-local"]')).toHaveCount(1); // the same tab, not a new one
  await expect(page.getByTestId('tab-state-local1')).toHaveAttribute('data-state', 'ok');

  // "do not ask again": the next restart goes without a question; Settings can switch it back on
  await rpc(bridge.url, 'script_write', { id: 'own:prog.sh', text: 'echo third-version-$$; sleep 120\n', eol: 'lf', bom: false, encoding: 'utf-8' });
  await page.getByTestId('tab-local1').click({ button: 'right' });
  await page.getByTestId('tabmenu-restart').click();
  await page.getByTestId('restart-never').check();
  await page.getByTestId('restart-ok').click();
  await expect.poll(() => screen(page), { timeout: 20_000 }).toContain('third-version-');
  expect(((await rpc(bridge.url, 'get_settings')) as { confirm_restart: boolean }).confirm_restart).toBe(false);
  await rpc(bridge.url, 'script_write', { id: 'own:prog.sh', text: 'echo fourth-version-$$; sleep 120\n', eol: 'lf', bom: false, encoding: 'utf-8' });
  await page.keyboard.press('Control+Shift+R'); // the shortcut, no dialog now
  await expect.poll(() => screen(page), { timeout: 20_000 }).toContain('fourth-version-');
  await expect(page.getByTestId('restart-dialog')).toHaveCount(0);
});

test('restart a finished program: no question', async ({ page }) => {
  await start(page);
  await runScript(page, 'echo quick-run-$$\n');
  await expect.poll(() => screen(page), { timeout: 20_000 }).toContain('quick-run-');
  await expect(page.getByTestId('tab-state-local1')).toHaveAttribute('data-state', 'off', { timeout: 20_000 });
  await page.getByTestId('tab-local1').click({ button: 'right' });
  await page.getByTestId('tabmenu-restart').click();
  await expect(page.getByTestId('restart-dialog')).toHaveCount(0);
  await expect.poll(async () => ((await screen(page)).match(/quick-run-/g) ?? []).length, { timeout: 20_000 }).toBeGreaterThan(0);
  await expect(page.locator('[data-testid^="tab-local"]')).toHaveCount(1);
});

test('tab menu: duplicate, close the others, close the tabs to the right', async ({ page }) => {
  await start(page);
  await page.getByTestId('rail-terminals').click();
  await page.getByTestId('profile-item').first().dblclick();
  await expect(page.getByTestId('tab-local1')).toBeVisible();
  // the Home tab has no menu (it cannot be closed or restarted)
  await page.getByTestId('tab-home').click({ button: 'right' });
  await expect(page.getByTestId('ctx-menu')).toHaveCount(0);
  // duplicate: a second terminal of the same shell
  await page.getByTestId('tab-local1').click({ button: 'right' });
  await page.getByTestId('tabmenu-duplicate').click();
  await expect(page.getByTestId('tab-local2')).toBeVisible();
  await page.getByTestId('tab-local2').click({ button: 'right' });
  await page.getByTestId('tabmenu-duplicate').click();
  await expect(page.getByTestId('tab-local3')).toBeVisible();
  // close the tabs to the right of the first: local2 and local3 (running programs: each is confirmed)
  await page.getByTestId('tab-local1').click({ button: 'right' });
  await page.getByTestId('tabmenu-close-right').click();
  await page.getByTestId('confirm-ok').click();
  await page.getByTestId('confirm-ok').click();
  await expect(page.getByTestId('tab-local2')).toHaveCount(0);
  await expect(page.getByTestId('tab-local3')).toHaveCount(0);
  await expect(page.getByTestId('tab-local1')).toBeVisible();
  // nothing to close on the right any more
  await page.getByTestId('tab-local1').click({ button: 'right' });
  await expect(page.getByTestId('tabmenu-close-right')).toBeDisabled();
  await expect(page.getByTestId('tabmenu-close-others')).toBeDisabled();
  await page.keyboard.press('Escape');
  await expect(page.getByTestId('ctx-menu')).toHaveCount(0);
});

test('tab menu of an SSH tab: restart the terminal, the explorer stays; the browser menu is not shown', async ({ page }) => {
  await start(page);
  await addServer(page, { name: 'Again', host: 'sshd-password', password: PASS });
  await connectFirst(page);
  await acceptHostKey(page);
  await expect(page.getByTestId('tab-s1')).toBeVisible();
  await expect(page.getByTestId('path-input')).toHaveValue('/home/tester');
  await page.getByTestId('tab-s1').click({ button: 'right' });
  await expect(page.getByTestId('tabmenu-restart')).toBeEnabled();
  await expect(page.getByTestId('tabmenu-duplicate')).toBeEnabled();
  await page.getByTestId('tabmenu-restart').click();
  await expect(page.getByTestId('restart-dialog')).toBeVisible();
  await expect(page.getByTestId('restart-dialog')).toContainText('The files and the explorer stay');
  await page.getByTestId('restart-ok').click();
  // a new shell on the same connection: the tab is alive, the explorer still shows the folder
  await expect(page.getByTestId('tab-state-s1')).toHaveAttribute('data-state', 'ok', { timeout: 20_000 });
  await expect(page.getByTestId('path-input')).toHaveValue('/home/tester');
  // the terminal is usable: a command runs (typed when the new shell shows its prompt)
  const text = () =>
    page.evaluate(() => {
      const e = document.querySelector('[data-testid="terminal"]') as unknown as { __term: XTerm };
      const t = e.__term.buffer.active;
      let s = '';
      for (let i = 0; i < t.length; i++) s += t.getLine(i)?.translateToString(true) + '\n';
      return s;
    });
  await expect.poll(text, { timeout: 20_000 }).toContain('Last login');
  await page.waitForTimeout(800); // the prompt after the greeting
  await page.locator('[data-testid="terminal"] textarea').first().focus();
  await page.keyboard.type('echo after-restart-$((6*7))\n', { delay: 30 });
  await expect.poll(text, { timeout: 20_000 }).toMatch(/^after-restart-42/m);
});
