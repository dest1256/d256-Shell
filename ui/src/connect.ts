// Connection flow: while `api.connect` is blocked in the backend, poll for questions (host key,
// password, verification code) and show them; handle refusals (changed key, wrong password).
// The same prompt watcher serves terminals (ssh asks through askpass while logging in).
import { api, ApiError, Connected, Prompt } from './api';
import { confirmDialog, h, modal } from './dom';

export interface ConnectOpts {
  label: string;
  serverId?: string;
  host?: string;
  port?: number;
  username?: string;
  /** Explicit password (e.g. typed in the server form); otherwise the saved one is used. */
  password?: string;
  testOnly?: boolean;
  /** Private key file / passphrase typed in the server form (before it is saved). */
  keyPath?: string;
  keyPassphrase?: string;
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

export function rid(): string {
  const b = new Uint8Array(8);
  crypto.getRandomValues(b);
  return Array.from(b, (x) => x.toString(16).padStart(2, '0')).join('');
}

function overlay(text: string, onCancel: () => void): { close(): void } {
  const cancel = h('button', { 'data-testid': 'connect-cancel', onclick: onCancel }, 'Cancel');
  const el = h(
    'div',
    { class: 'modal-back', 'data-testid': 'connecting' },
    h('div', { class: 'modal progress', role: 'status' }, h('div', { class: 'spinner' }), h('div', { class: 'grow' }, text), cancel),
  );
  document.body.append(el);
  return { close: () => el.remove() };
}

async function hostKeyDialog(p: Prompt, closers: Array<() => void>): Promise<boolean> {
  const r = await modal<boolean>((close) => {
    closers.push(() => close(null));
    return h(
      'div',
      { role: 'dialog', 'data-testid': 'hostkey-dialog' },
      h('h3', {}, 'New server: check the key'),
      h('p', {}, `You are connecting to ${p.host}:${p.port} for the first time. Compare the key fingerprint with the one the server administrator gave you.`),
      h('div', { class: 'kv' }, h('span', {}, 'Key type'), h('code', {}, p.key_type ?? '')),
      h('div', { class: 'kv' }, h('span', {}, 'Fingerprint'), h('code', { class: 'fp', 'data-testid': 'fingerprint' }, p.fingerprint ?? '')),
      h(
        'div',
        { class: 'row end' },
        h('button', { onclick: () => close(false), 'data-testid': 'hostkey-reject' }, 'Cancel'),
        h('button', { class: 'primary', onclick: () => close(true), 'data-testid': 'hostkey-accept' }, 'Trust and connect'),
      ),
    );
  });
  return r === true;
}

interface InputResult {
  value: string;
  update: boolean;
}

async function inputDialog(p: Prompt, closers: Array<() => void>, canUpdate: boolean): Promise<InputResult | null> {
  return modal<InputResult>((close) => {
    closers.push(() => close(null));
    const input = h('input', { type: p.echo ? 'text' : 'password', autocomplete: 'off', 'data-testid': 'prompt-input' }) as HTMLInputElement;
    // the passphrase of a key file is not the account password: nothing to "update" in the saved server password
    const isPassphrase = /passphrase/i.test(p.text ?? '');
    const offerUpdate = canUpdate && !!p.is_password && !!p.saved_password_rejected && !isPassphrase;
    const update = h('input', { type: 'checkbox', checked: true, 'data-testid': 'prompt-update' }) as HTMLInputElement;
    return h(
      'form',
      { role: 'dialog', 'data-testid': 'prompt-dialog', onsubmit: (e) => (e.preventDefault(), close({ value: input.value, update: offerUpdate && update.checked })) },
      h('h3', {}, isPassphrase ? 'Key passphrase' : p.is_password ? 'Password required' : 'Input required'),
      p.saved_password_rejected ? h('p', { class: 'warn' }, isPassphrase ? 'The saved passphrase was not accepted.' : 'The saved password was not accepted. Repeated attempts are not made automatically.') : null,
      h('p', { 'data-testid': 'prompt-text' }, p.text ?? ''),
      input,
      offerUpdate ? h('label', { class: 'check' }, update, 'Update the saved server password') : null,
      h(
        'div',
        { class: 'row end' },
        h('button', { type: 'button', onclick: () => close(null), 'data-testid': 'prompt-cancel' }, 'Cancel'),
        h('button', { type: 'submit', class: 'primary', 'data-testid': 'prompt-ok' }, 'Continue'),
      ),
    );
  });
}

async function handlePrompt(p: Prompt, closers: Array<() => void>, serverId?: string): Promise<void> {
  try {
    if (p.kind === 'host_key') {
      await api.answerPrompt(p.id, { accept: await hostKeyDialog(p, closers) });
    } else {
      const r = await inputDialog(p, closers, !!serverId);
      // answering with nothing = the user refused; the backend then stops the login (no empty password is sent)
      await api.answerPrompt(p.id, r === null ? {} : { value: r.value });
      if (r?.update && serverId) await api.setServerPassword(serverId, r.value);
    }
  } catch {
    /* prompt already closed by the backend (cancel/timeout) */
  }
}

/**
 * Poll the backend for questions of `attempt` (a connect attempt or a terminal id) and show dialogs.
 * Stops itself after `maxMs`; the returned function stops it earlier and closes open dialogs.
 */
export function watchPrompts(attempt: string, opts: { serverId?: string; maxMs?: number } = {}): () => void {
  const closers: Array<() => void> = [];
  const shown = new Set<number>();
  let stop = false;
  const until = Date.now() + (opts.maxMs ?? 24 * 3600 * 1000);
  void (async () => {
    while (!stop && Date.now() < until) {
      try {
        for (const p of await api.pollPrompts(attempt)) {
          if (!shown.has(p.id)) {
            shown.add(p.id);
            void handlePrompt(p, closers, opts.serverId);
          }
        }
      } catch {
        /* transient */
      }
      await sleep(200);
    }
  })();
  return () => {
    stop = true;
    closers.forEach((c) => c());
  };
}

/** Returns true if the user replaced the key and the connection should be retried. */
async function changedKeyDialog(e: ApiError, attempt: string): Promise<boolean> {
  const d = (e.details ?? {}) as { host?: string; port?: number; key_type?: string; fingerprint?: string; known_types?: string[] };
  const replace = await modal<boolean>((close) =>
    h(
      'div',
      { role: 'alertdialog', class: 'danger-box', 'data-testid': 'hostkey-changed' },
      h('h3', {}, '⚠ The server key has changed!'),
      h('p', { class: 'danger-text' }, 'A man-in-the-middle attack is possible. The connection was NOT made, the password was not sent.'),
      h('div', { class: 'kv' }, h('span', {}, 'Server'), h('code', {}, `${d.host}:${d.port}`)),
      h('div', { class: 'kv' }, h('span', {}, 'New key'), h('code', { class: 'fp', 'data-testid': 'fingerprint' }, `${d.key_type} ${d.fingerprint}`)),
      h('div', { class: 'kv' }, h('span', {}, 'Previously known types'), h('code', {}, (d.known_types ?? []).join(', '))),
      h('p', {}, 'If you are sure the server key was changed on purpose (reinstall, migration), you can replace it. Otherwise close the window and contact the administrator.'),
      h(
        'div',
        { class: 'row end' },
        h('button', { class: 'primary', onclick: () => close(false), 'data-testid': 'changed-close' }, 'Close'),
        h('button', { class: 'danger', onclick: () => close(true), 'data-testid': 'changed-replace' }, 'Replace key…'),
      ),
    ),
  );
  if (!replace) return false;
  const sure = await confirmDialog(
    'Replace the saved server key?',
    `Old key for ${d.host}:${d.port} will be removed from known_hosts and the new one (${d.fingerprint}) will become trusted. This action cannot be undone.`,
    'Yes, replace the key',
    true,
  );
  if (!sure) return false;
  await api.replaceHostKey(attempt);
  return true;
}

async function wrongPasswordDialog(canSave: boolean): Promise<{ password: string; save: boolean } | null> {
  return modal<{ password: string; save: boolean }>((close) => {
    const input = h('input', { type: 'password', autocomplete: 'off', 'data-testid': 'retry-password' }) as HTMLInputElement;
    const save = h('input', { type: 'checkbox', checked: canSave, 'data-testid': 'retry-save' }) as HTMLInputElement;
    return h(
      'form',
      { role: 'dialog', 'data-testid': 'wrong-password', onsubmit: (e) => (e.preventDefault(), close({ password: input.value, save: save.checked })) },
      h('h3', {}, 'Wrong password'),
      h('p', {}, 'The server rejected the saved password. To avoid locking the account, it was not retried automatically. Enter the password manually.'),
      input,
      canSave ? h('label', { class: 'check' }, save, 'Save this password after a successful login') : null,
      h(
        'div',
        { class: 'row end' },
        h('button', { type: 'button', onclick: () => close(null), 'data-testid': 'retry-cancel' }, 'Cancel'),
        h('button', { type: 'submit', class: 'primary', 'data-testid': 'retry-ok' }, 'Connect'),
      ),
    );
  });
}

export async function errorDialog(title: string, e: unknown): Promise<void> {
  const err = e instanceof ApiError ? e : null;
  const detail = err ? `${err.code}${err.details ? '\n' + JSON.stringify(err.details, null, 2) : ''}` : String(e);
  await modal<boolean>((close) => {
    const pre = h('pre', { class: 'detail' }, detail);
    pre.style.display = 'none';
    return h(
      'div',
      { role: 'alertdialog', 'data-testid': 'error-dialog' },
      h('h3', {}, title),
      h('p', { 'data-testid': 'error-message' }, err ? err.message : String(e)),
      h('button', { class: 'link left', onclick: () => (pre.style.display = pre.style.display === 'none' ? 'block' : 'none') }, 'Details'),
      pre,
      h('div', { class: 'row end' }, h('button', { class: 'primary', onclick: () => close(true), 'data-testid': 'error-ok' }, 'OK')),
    );
  });
}

/** Run the whole connection procedure. Resolves with the session, or null when cancelled/failed (already reported). */
export async function connectFlow(o: ConnectOpts): Promise<Connected | null> {
  let password = o.password;
  let save = false;
  for (;;) {
    const attempt = rid();
    const ov = overlay(`Connecting to ${o.label}…`, () => void api.cancelConnect(attempt));
    const stopWatch = watchPrompts(attempt);
    try {
      return await api.connect({
        attempt,
        server_id: o.serverId,
        host: o.host,
        port: o.port,
        username: o.username,
        password,
        save_password: save,
        test_only: o.testOnly,
        key_path: o.keyPath,
        key_passphrase: o.keyPassphrase,
      });
    } catch (e) {
      stopWatch();
      ov.close();
      if (!(e instanceof ApiError)) {
        await errorDialog('Could not connect', e);
        return null;
      }
      if (e.code === 'cancelled' || e.code === 'host_key_rejected' || e.code === 'auth_cancelled') return null;
      if (e.code === 'host_key_changed') {
        if (await changedKeyDialog(e, attempt)) continue;
        return null;
      }
      if (e.code === 'auth_failed' && (e.details as { saved_password_used?: boolean } | undefined)?.saved_password_used) {
        const r = await wrongPasswordDialog(!!o.serverId);
        if (!r) return null;
        password = r.password;
        save = r.save;
        continue;
      }
      await errorDialog(e.code === 'host_key_revoked' ? 'Server key revoked' : e.code === 'key_error' || e.code === 'key_rejected' ? 'Key login failed' : 'Could not connect', e);
      return null;
    } finally {
      stopWatch();
      ov.close();
    }
  }
}
