import { api, ServerView, Settings, SshInfo } from './api';
import { connectFlow } from './connect';
import { h, modal, toast } from './dom';
import { SCHEMES } from './schemes';
import { getSettings, updateSettings } from './settings';
import { aboutDialog } from './about';
import { native } from './native';
import { pwField, errText } from './util';
import { applyLanguage, LANGUAGES, t, tm } from './i18n';

export async function serverForm(existing?: ServerView): Promise<boolean> {
  const f = (id: string, label: string, value = '', type = 'text') =>
    h('label', {}, label, h('input', { id, type, value, 'data-testid': id, autocomplete: 'off' }));
  const val = (id: string) => (document.getElementById(id) as HTMLInputElement).value;
  const err = h('div', { class: 'error', 'data-testid': 'form-error' });
  let pwShown = false;

  const res = await modal<boolean>((close) => {
    const pwInput = h('input', {
      id: 'f-password',
      type: 'password',
      'data-testid': 'f-password',
      autocomplete: 'new-password',
      placeholder: existing?.has_password ? t('•••••••• (do not change)') : '',
    }) as HTMLInputElement;
    const eye = h(
      'button',
      {
        type: 'button',
        class: 'btn',
        'data-testid': 'eye',
        onclick: async () => {
          pwShown = !pwShown;
          if (pwShown && existing && !pwInput.value) {
            pwInput.value = (await api.getPassword(existing.id).catch(() => null)) ?? '';
          }
          pwInput.type = pwShown ? 'text' : 'password';
          eye.textContent = pwShown ? t('Hide') : t('Show');
        },
      },
      t('Show'),
    );
    // private key login (like "Use private key" in MobaXterm): a checkbox, the key file and, if wanted, its passphrase
    const useKey = h('input', { type: 'checkbox', 'data-testid': 'f-usekey' }) as HTMLInputElement;
    useKey.checked = !!existing?.key_path;
    const keyPath = h('input', { id: 'f-keypath', type: 'text', 'data-testid': 'f-keypath', value: existing?.key_path ?? '', spellcheck: 'false', placeholder: t('Private key file (id_ed25519, id_rsa, *.pem)'), autocomplete: 'off' }) as HTMLInputElement;
    const keyPass = h('input', { id: 'f-keypass', type: 'password', 'data-testid': 'f-keypass', autocomplete: 'new-password', placeholder: existing?.has_key_passphrase ? t('•••••••• (do not change)') : t('Key passphrase (optional)') }) as HTMLInputElement;
    const browse = h('button', { type: 'button', class: 'btn', 'data-testid': 'f-keybrowse', onclick: async () => {
      const [p] = await native.pick('files');
      if (p) {
        keyPath.value = p;
        useKey.checked = true;
        syncKey();
      }
    } }, t('Browse…'));
    const keyBox = h('div', { class: 'col key-box', 'data-testid': 'key-box' }, h('div', { class: 'row' }, keyPath, browse), keyPass, h('p', { class: 'muted small' }, t('PuTTY files (.ppk) are not supported: export the key to OpenSSH format. The password, if given, is used only if the server rejects the key.')));
    const syncKey = () => (keyBox.style.display = useKey.checked ? '' : 'none');
    useKey.addEventListener('change', syncKey);
    syncKey();
    const save = async () => {
      err.textContent = '';
      try {
        const password = pwInput.value;
        await api.saveServer({
          id: existing?.id,
          name: val('f-name'),
          host: val('f-host').trim(),
          port: Number(val('f-port')),
          username: val('f-user').trim(),
          // empty field while editing => keep the stored password
          password: existing && password === '' ? undefined : password,
          initial_dir: val('f-dir'),
          ssh_extra_args: val('f-args'),
          // unchecked = no key (the saved passphrase is dropped by the backend)
          key_path: useKey.checked ? keyPath.value.trim() : '',
          key_passphrase: useKey.checked && keyPass.value !== '' ? keyPass.value : undefined,
        });
        close(true);
      } catch (e) {
        err.textContent = errText(e);
      }
    };
    return h(
      'form',
      { 'data-testid': 'server-form', onsubmit: (e) => (e.preventDefault(), void save()) },
      h('h3', {}, existing ? t('Edit server') : t('New server')),
      f('f-name', t('Name'), existing?.name),
      h('div', { class: 'row' }, f('f-host', t('Address'), existing?.host), f('f-port', t('Port'), String(existing?.port ?? 22), 'number')),
      f('f-user', t('User'), existing?.username),
      h('label', {}, t('Password'), h('div', { class: 'row' }, pwInput, eye)),
      h('label', { class: 'check' }, useKey, ' ' + t('Use a private key')),
      keyBox,
      f('f-dir', t('Start folder (optional)'), existing?.initial_dir ?? ''),
      f('f-args', t('Extra ssh arguments (optional)'), existing?.ssh_extra_args ?? ''),
      err,
      h(
        'div',
        { class: 'row end' },
        h(
          'button',
          {
            type: 'button',
            class: 'btn left-auto',
            'data-testid': 'form-test',
            title: t('Check the SFTP login without opening a terminal'),
            onclick: async () => {
              err.textContent = '';
              const host = val('f-host').trim();
              const username = val('f-user').trim();
              const port = Number(val('f-port'));
              if (!host || !username || !port) return void (err.textContent = t('Fill in the address, port and user.'));
              const password = pwInput.value || (existing ? ((await api.getPassword(existing.id).catch(() => null)) ?? '') : '');
              const r = await connectFlow({ label: `${username}@${host}`, host, port, username, password: password || undefined, testOnly: true, keyPath: useKey.checked ? keyPath.value.trim() : undefined, keyPassphrase: useKey.checked && keyPass.value ? keyPass.value : undefined });
              if (r) toast(t('Connection successful: {0}@{1}', username, host));
            },
          },
          t('Test connection'),
        ),
        h('button', { type: 'button', class: 'btn', onclick: () => close(false), 'data-testid': 'form-cancel' }, t('Cancel')),
        h('button', { type: 'submit', class: 'btn primary', 'data-testid': 'form-save' }, t('Save')),
      ),
    );
  });
  return res === true;
}

export async function changeMasterDialog(): Promise<void> {
  const err = h('div', { class: 'error', 'data-testid': 'cm-error' });
  await modal<boolean>((close) => {
    const o = pwField('cm-old', t('Current master password'));
    const n1 = pwField('cm-new1', t('New master password'), 'new-password');
    const n2 = pwField('cm-new2', t('Repeat the new password'), 'new-password');
    return h(
      'form',
      {
        onsubmit: async (e) => {
          e.preventDefault();
          if (n1.value !== n2.value) return void (err.textContent = t('The passwords do not match.'));
          try {
            await api.changeMaster(o.value, n1.value);
            toast(t('Master password changed.'));
            close(true);
          } catch (x) {
            err.textContent = errText(x);
          }
        },
      },
      h('h3', {}, t('Change master password')),
      o,
      n1,
      n2,
      err,
      h(
        'div',
        { class: 'row end' },
        h('button', { type: 'button', class: 'btn', onclick: () => close(false) }, t('Cancel')),
        h('button', { type: 'submit', class: 'btn primary', 'data-testid': 'cm-ok' }, t('Edit')),
      ),
    );
  });
}

const AUTO_MODES: Array<[string, string]> = [
  ['auto', t('Automatic (askpass for ssh 8.4+, otherwise careful typing into the terminal)')],
  ['askpass', t('Only askpass (SSH_ASKPASS)')],
  ['expect', t('Only careful typing into the terminal (for old ssh)')],
  ['off', t('Do not fill in the password automatically')],
];

/** Settings: ssh location and password mode, terminal appearance and behaviour. */
export async function settingsDialog(): Promise<void> {
  const [info, cur] = await Promise.all([api.sshInfo().catch(() => ({ found: false }) as SshInfo), Promise.resolve(getSettings())]);
  const field = (label: string, el: HTMLElement) => h('label', {}, label, el);
  const sel = (id: string, opts: Array<[string, string]>, value: string) => {
    const s = h('select', { id, 'data-testid': id }) as HTMLSelectElement;
    for (const [v, t] of opts) s.append(h('option', { value: v, selected: v === value }, t));
    return s;
  };
  const num = (id: string, v: number) => h('input', { id, type: 'number', value: String(v), 'data-testid': id }) as HTMLInputElement;
  const text = (id: string, v: string) => h('input', { id, type: 'text', value: v, 'data-testid': id, spellcheck: 'false' }) as HTMLInputElement;

  const lang = sel('set-lang', [['auto', t('System default')], ...LANGUAGES.map((l) => [l.code, l.name] as [string, string])], cur.language);
  const profs = await api.shellProfiles().then((r) => r.profiles).catch(() => []);
  const tabsPos = sel('set-tabs', [['top', t('Top')], ['left', t('Left (vertical)')]], cur.tabs_position);
  const startup = sel('set-startup', [['home', t('Home')], ...profs.map((p) => [p.id, p.name] as [string, string])], cur.startup_profile);
  const sshPath = text('set-ssh-path', cur.ssh_path);
  sshPath.placeholder = t('empty: detect automatically');
  const mode = sel('set-auto', AUTO_MODES, cur.auto_password);
  const scheme = sel('set-scheme', SCHEMES.map((s) => [s.id, s.name] as [string, string]), cur.scheme);
  const cursor = sel('set-cursor', [['bar', t('Bar')], ['block', t('Block')], ['underline', t('Underline')]], cur.cursor_style);
  const rclick = sel('set-rclick', [['copy-paste', t('Copy the selection / paste')], ['menu', t('Browser context menu')]], cur.right_click);
  const wheel = sel('set-wheel', [['arrows', t('Up/down arrows (btop, htop, mc)')], ['mouse', t('Mouse events (tmux, vim with mouse)')]], cur.wheel_mode);
  const cloneMode = sel('set-clone', [['ask', t('Show the commands and ask')], ['auto', t('Repeat the folder, environment and program at once')], ['view', t('Output only, no commands')]], cur.clone_mode);
  const size = num('set-font-size', cur.font_size);
  const scroll = num('set-scrollback', cur.scrollback);
  const family = text('set-font', cur.font_family);
  const parallel = num('set-parallel', cur.parallel_transfers);
  const dragLimit = num('set-dragout', cur.drag_out_limit_mb);
  const editLimit = num('set-editlimit', cur.editor_limit_mb);
  const autoLock = num('set-autolock', cur.auto_lock_minutes);
  const monInterval = num('set-mon-interval', cur.monitor_interval_ms);
  const monOn = h('input', { type: 'checkbox', id: 'set-mon', 'data-testid': 'set-mon' }) as HTMLInputElement;
  monOn.checked = cur.monitor_enabled;
  const keepMtime = h('input', { type: 'checkbox', id: 'set-mtime', 'data-testid': 'set-mtime' }) as HTMLInputElement;
  keepMtime.checked = cur.keep_mtime;
  const err = h('div', { class: 'error', 'data-testid': 'settings-error' });

  const sshLine = info.found
    ? t('Found: {0}\n{1}\nAuto password: {2}', info.path, info.version, info.mode === 'askpass' ? 'askpass (SSH_ASKPASS)' : info.mode === 'expect' ? t('careful typing into the terminal') : 'off')
    : t('ssh not found: {0}', tm(info.error ?? ''));

  await modal<boolean>((close) =>
    h(
      'form',
      {
        'data-testid': 'settings-dialog',
        onsubmit: async (e) => {
          e.preventDefault();
          try {
            await updateSettings({
              ssh_path: sshPath.value.trim(),
              auto_password: mode.value as Settings['auto_password'],
              scheme: scheme.value,
              cursor_style: cursor.value as Settings['cursor_style'],
              right_click: rclick.value as Settings['right_click'],
              wheel_mode: wheel.value as Settings['wheel_mode'],
              clone_mode: cloneMode.value as Settings['clone_mode'],
              font_size: Number(size.value),
              scrollback: Number(scroll.value),
              font_family: family.value.trim() || getSettings().font_family,
              parallel_transfers: Number(parallel.value),
              drag_out_limit_mb: Number(dragLimit.value),
              editor_limit_mb: Number(editLimit.value),
              auto_lock_minutes: Number(autoLock.value),
              keep_mtime: keepMtime.checked,
              monitor_enabled: monOn.checked,
              monitor_interval_ms: Number(monInterval.value),
              language: lang.value,
              tabs_position: tabsPos.value,
              startup_profile: startup.value,
            });
            close(true);
            if (applyLanguage(lang.value)) location.reload();
          } catch (x) {
            err.textContent = String(x instanceof Error ? x.message : x);
          }
        },
      },
      h('h3', {}, t('Settings')),
      field(t('Language'), lang),
      h('h4', {}, t('Tabs and start')),
      h('div', { class: 'row' }, field(t('Tabs'), tabsPos), field(t('Open at start'), startup)),
      h('h4', {}, t('System ssh')),
      h('pre', { class: 'detail', style: 'display:block', 'data-testid': 'ssh-info' }, sshLine),
      field(t('Path to ssh (ssh.exe on Windows)'), sshPath),
      field(t('Fill in the password automatically'), mode),
      h('h4', {}, t('Terminal')),
      h('div', { class: 'row' }, field(t('Colour scheme'), scheme), field(t('Cursor'), cursor)),
      h('div', { class: 'row' }, field(t('Font size'), size), field(t('Scrollback lines'), scroll)),
      field(t('Font'), family),
      field(t('Right mouse button'), rclick),
      field(t('Wheel in full-screen programs'), wheel),
      field(t('Duplicate terminal (Ctrl+Shift+D)'), cloneMode),
      h('h4', {}, t('Transfers and editor')),
      h('div', { class: 'row' }, field(t('Parallel transfers'), parallel), field(t('Drag-out limit, MB'), dragLimit)),
      h('div', { class: 'row' }, field(t('File size limit in the editor, MB'), editLimit), field(t('Auto-lock, minutes (0 = off)'), autoLock)),
      h('label', { class: 'check' }, keepMtime, ' ' + t('Keep the modification time of files on transfer')),
      h('h4', {}, t('Server monitor')),
      h('label', { class: 'check' }, monOn, ' ' + t('Show server load under the terminal (reads /proc over SFTP, no commands are run)')),
      field(t('Refresh interval, ms (from 100)'), monInterval),
      err,
      h('div', { class: 'row end' }, h('button', { type: 'button', class: 'left-auto', onclick: () => void aboutDialog(), 'data-testid': 'open-about' }, t('About')), h('button', { type: 'button', onclick: () => close(false), 'data-testid': 'settings-cancel' }, t('Cancel')), h('button', { type: 'submit', class: 'primary', 'data-testid': 'settings-save' }, t('Save'))),
    ),
  );
}
