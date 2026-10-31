import './style.css';
import { api, ApiError } from './api';
import { clear, h, toast } from './dom';
import { icon } from './icons';
import { showShell } from './shell';
import { Titlebar } from './titlebar';
import { loadSettings } from './settings';
import { errText, pwField } from './util';

const app = document.getElementById('app')!;
const titlebar = new Titlebar();
const screen = h('div', { class: 'screen' });
app.append(titlebar.el, screen);

function authTabs(): void {
  titlebar.setTabs([{ id: 'auth', title: 'd256 Shell', icon: 'lock' }], 'auth', { onSelect: () => undefined });
}

function logo(): HTMLElement {
  return h('div', { class: 'logo' }, icon('terminal', 28));
}

async function openShell(): Promise<void> {
  await showShell({ screen, titlebar, onLock: () => showUnlock(false) });
}

// ---------- master password: create ----------
function showSetup(): void {
  authTabs();
  clear(screen);
  const p1 = pwField('master1', 'Master password (at least 8 characters)', 'new-password');
  const p2 = pwField('master2', 'Repeat the master password', 'new-password');
  const err = h('div', { class: 'error', 'data-testid': 'error' });
  const submit = async () => {
    err.textContent = '';
    if (p1.value !== p2.value) return void (err.textContent = 'The passwords do not match.');
    try {
      await api.createVault(p1.value);
      await openShell();
    } catch (e) {
      err.textContent = errText(e);
    }
  };
  screen.append(
    h(
      'div',
      { class: 'auth' },
      h(
        'form',
        { class: 'auth-card', onsubmit: (e) => (e.preventDefault(), void submit()) },
        logo(),
        h('h1', {}, 'Create a master password'),
        h('p', {}, 'It encrypts the passwords of your servers. The data folder can be moved to another computer.'),
        h('p', { class: 'warn' }, 'The master password cannot be recovered: if you forget it, the saved passwords will be lost.'),
        p1,
        p2,
        err,
        h('button', { class: 'primary', type: 'submit', 'data-testid': 'create' }, 'Create vault'),
      ),
    ),
  );
  p1.focus();
}

// ---------- master password: unlock ----------
function showUnlock(backupExists: boolean, note = ''): void {
  authTabs();
  clear(screen);
  const pw = pwField('master', 'Master password');
  const err = h('div', { class: 'error', 'data-testid': 'error' }, note);
  const actions = h('div', { class: 'col' });
  const offerRestore = () => {
    clear(actions);
    actions.append(
      h(
        'button',
        {
          type: 'button',
          'data-testid': 'restore',
          onclick: async () => {
            try {
              await api.restoreBackup(pw.value);
              toast('The vault was restored from the backup.');
              await openShell();
            } catch (e) {
              err.textContent = errText(e);
            }
          },
        },
        'Restore from the backup (vault.bin.bak)',
      ),
    );
  };
  const submit = async () => {
    err.textContent = '';
    try {
      await api.unlock(pw.value);
      await openShell();
    } catch (e) {
      err.textContent = errText(e);
      if (e instanceof ApiError && (e.code === 'vault_corrupt' || e.code === 'vault_missing') && backupExists) offerRestore();
    }
  };
  screen.append(
    h(
      'div',
      { class: 'auth' },
      h(
        'form',
        { class: 'auth-card', onsubmit: (e) => (e.preventDefault(), void submit()) },
        logo(),
        h('h1', {}, 'd256 Shell'),
        h('p', {}, 'Enter the master password to open the vault.'),
        pw,
        err,
        h('button', { class: 'primary', type: 'submit', 'data-testid': 'unlock' }, 'Unlock'),
        actions,
      ),
    ),
  );
  if (note && backupExists) offerRestore();
  pw.focus();
}

async function start(): Promise<void> {
  await loadSettings().catch(() => undefined);
  try {
    const st = await api.status();
    if (st.state === 'no_vault') return showSetup();
    if (st.state === 'unlocked') return void (await openShell());
    showUnlock(st.backup_exists);
  } catch (e) {
    screen.textContent = `Start error: ${errText(e)}`;
  }
}

void start();
