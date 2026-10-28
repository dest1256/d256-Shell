import { api, ServerView, Settings, SshInfo } from './api';
import { connectFlow } from './connect';
import { h, modal, toast } from './dom';
import { SCHEMES } from './schemes';
import { getSettings, updateSettings } from './settings';
import { aboutDialog } from './about';
import { native } from './native';
import { pwField, errText } from './util';

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
      placeholder: existing?.has_password ? '•••••••• (не менять)' : '',
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
          eye.textContent = pwShown ? 'Скрыть' : 'Показать';
        },
      },
      'Показать',
    );
    // private key login (like "Use private key" in MobaXterm): a checkbox, the key file and, if wanted, its passphrase
    const useKey = h('input', { type: 'checkbox', 'data-testid': 'f-usekey' }) as HTMLInputElement;
    useKey.checked = !!existing?.key_path;
    const keyPath = h('input', { id: 'f-keypath', type: 'text', 'data-testid': 'f-keypath', value: existing?.key_path ?? '', spellcheck: 'false', placeholder: 'Файл приватного ключа (id_ed25519, id_rsa, *.pem)', autocomplete: 'off' }) as HTMLInputElement;
    const keyPass = h('input', { id: 'f-keypass', type: 'password', 'data-testid': 'f-keypass', autocomplete: 'new-password', placeholder: existing?.has_key_passphrase ? '•••••••• (не менять)' : 'Парольная фраза ключа (необязательно)' }) as HTMLInputElement;
    const browse = h('button', { type: 'button', class: 'btn', 'data-testid': 'f-keybrowse', onclick: async () => {
      const [p] = await native.pick('files');
      if (p) {
        keyPath.value = p;
        useKey.checked = true;
        syncKey();
      }
    } }, 'Обзор…');
    const keyBox = h('div', { class: 'col key-box', 'data-testid': 'key-box' }, h('div', { class: 'row' }, keyPath, browse), keyPass, h('p', { class: 'muted small' }, 'Файлы PuTTY (.ppk) не поддерживаются: экспортируйте ключ в формат OpenSSH. Пароль, если он указан, будет использован только если сервер отклонит ключ.'));
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
      h('h3', {}, existing ? 'Изменить сервер' : 'Новый сервер'),
      f('f-name', 'Название', existing?.name),
      h('div', { class: 'row' }, f('f-host', 'Адрес', existing?.host), f('f-port', 'Порт', String(existing?.port ?? 22), 'number')),
      f('f-user', 'Пользователь', existing?.username),
      h('label', {}, 'Пароль', h('div', { class: 'row' }, pwInput, eye)),
      h('label', { class: 'check' }, useKey, ' Использовать приватный ключ'),
      keyBox,
      f('f-dir', 'Стартовая папка (необязательно)', existing?.initial_dir ?? ''),
      f('f-args', 'Доп. аргументы ssh (необязательно)', existing?.ssh_extra_args ?? ''),
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
            title: 'Проверка входа по SFTP, без открытия терминала',
            onclick: async () => {
              err.textContent = '';
              const host = val('f-host').trim();
              const username = val('f-user').trim();
              const port = Number(val('f-port'));
              if (!host || !username || !port) return void (err.textContent = 'Заполните адрес, порт и пользователя.');
              const password = pwInput.value || (existing ? ((await api.getPassword(existing.id).catch(() => null)) ?? '') : '');
              const r = await connectFlow({ label: `${username}@${host}`, host, port, username, password: password || undefined, testOnly: true, keyPath: useKey.checked ? keyPath.value.trim() : undefined, keyPassphrase: useKey.checked && keyPass.value ? keyPass.value : undefined });
              if (r) toast(`Подключение успешно: ${username}@${host}`);
            },
          },
          'Проверить подключение',
        ),
        h('button', { type: 'button', class: 'btn', onclick: () => close(false), 'data-testid': 'form-cancel' }, 'Отмена'),
        h('button', { type: 'submit', class: 'btn primary', 'data-testid': 'form-save' }, 'Сохранить'),
      ),
    );
  });
  return res === true;
}

export async function changeMasterDialog(): Promise<void> {
  const err = h('div', { class: 'error', 'data-testid': 'cm-error' });
  await modal<boolean>((close) => {
    const o = pwField('cm-old', 'Текущий мастер-пароль');
    const n1 = pwField('cm-new1', 'Новый мастер-пароль', 'new-password');
    const n2 = pwField('cm-new2', 'Повторите новый пароль', 'new-password');
    return h(
      'form',
      {
        onsubmit: async (e) => {
          e.preventDefault();
          if (n1.value !== n2.value) return void (err.textContent = 'Пароли не совпадают.');
          try {
            await api.changeMaster(o.value, n1.value);
            toast('Мастер-пароль изменён.');
            close(true);
          } catch (x) {
            err.textContent = errText(x);
          }
        },
      },
      h('h3', {}, 'Смена мастер-пароля'),
      o,
      n1,
      n2,
      err,
      h(
        'div',
        { class: 'row end' },
        h('button', { type: 'button', class: 'btn', onclick: () => close(false) }, 'Отмена'),
        h('button', { type: 'submit', class: 'btn primary', 'data-testid': 'cm-ok' }, 'Изменить'),
      ),
    );
  });
}

const AUTO_MODES: Array<[string, string]> = [
  ['auto', 'Автоматически (askpass для ssh 8.4+, иначе осторожный ввод в терминал)'],
  ['askpass', 'Только askpass (SSH_ASKPASS)'],
  ['expect', 'Только осторожный ввод в терминал (для старых ssh)'],
  ['off', 'Не подставлять пароль автоматически'],
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

  const sshPath = text('set-ssh-path', cur.ssh_path);
  sshPath.placeholder = 'пусто — искать автоматически';
  const mode = sel('set-auto', AUTO_MODES, cur.auto_password);
  const scheme = sel('set-scheme', SCHEMES.map((s) => [s.id, s.name] as [string, string]), cur.scheme);
  const cursor = sel('set-cursor', [['bar', 'Черта'], ['block', 'Блок'], ['underline', 'Подчёркивание']], cur.cursor_style);
  const rclick = sel('set-rclick', [['copy-paste', 'Копировать выделенное / вставить'], ['menu', 'Контекстное меню браузера']], cur.right_click);
  const wheel = sel('set-wheel', [['arrows', 'Стрелки вверх/вниз (btop, htop, mc)'], ['mouse', 'События мыши (tmux, vim с mouse)']], cur.wheel_mode);
  const cloneMode = sel('set-clone', [['ask', 'Показать команды и спросить'], ['auto', 'Сразу повторить папку, окружение и программу'], ['view', 'Только вывод, без команд']], cur.clone_mode);
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
    ? `Найден: ${info.path}\n${info.version}\nАвтопароль: ${info.mode === 'askpass' ? 'askpass (SSH_ASKPASS)' : info.mode === 'expect' ? 'осторожный ввод в терминал' : 'выключен'}`
    : `ssh не найден: ${info.error ?? ''}`;

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
            });
            close(true);
          } catch (x) {
            err.textContent = String(x instanceof Error ? x.message : x);
          }
        },
      },
      h('h3', {}, 'Настройки'),
      h('h4', {}, 'Системный ssh'),
      h('pre', { class: 'detail', style: 'display:block', 'data-testid': 'ssh-info' }, sshLine),
      field('Путь к ssh (ssh.exe в Windows)', sshPath),
      field('Автоподстановка пароля', mode),
      h('h4', {}, 'Терминал'),
      h('div', { class: 'row' }, field('Цветовая схема', scheme), field('Курсор', cursor)),
      h('div', { class: 'row' }, field('Размер шрифта', size), field('Строк прокрутки', scroll)),
      field('Шрифт', family),
      field('Правая кнопка мыши', rclick),
      field('Колесо в полноэкранных программах', wheel),
      field('Дублирование терминала (Ctrl+Shift+D)', cloneMode),
      h('h4', {}, 'Передачи и редактор'),
      h('div', { class: 'row' }, field('Параллельных передач', parallel), field('Лимит перетаскивания наружу, МБ', dragLimit)),
      h('div', { class: 'row' }, field('Лимит файла в редакторе, МБ', editLimit), field('Автоблокировка, минут (0 — выкл.)', autoLock)),
      h('label', { class: 'check' }, keepMtime, ' Сохранять время изменения файлов при передаче'),
      h('h4', {}, 'Монитор сервера'),
      h('label', { class: 'check' }, monOn, ' Показывать загрузку сервера под терминалом (читает /proc по SFTP, команды не выполняются)'),
      field('Интервал обновления, мс (от 100)', monInterval),
      err,
      h('div', { class: 'row end' }, h('button', { type: 'button', class: 'left-auto', onclick: () => void aboutDialog(), 'data-testid': 'open-about' }, 'О программе'), h('button', { type: 'button', onclick: () => close(false), 'data-testid': 'settings-cancel' }, 'Отмена'), h('button', { type: 'submit', class: 'primary', 'data-testid': 'settings-save' }, 'Сохранить')),
    ),
  );
}
