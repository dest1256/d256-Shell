// Main window layout (MobaXterm-like): toolbar, left rail + sessions sidebar, tabbed work area, status bar.
import { EditorPane, pickEncoding } from './editor';
import { prepareClone } from './clone';
import { PaneSet } from './panes';
import { api, ApiError, Connected, inTauri, ServerView } from './api';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { connectFlow } from './connect';
import { changeMasterDialog, serverForm, settingsDialog } from './dialogs';
import { clear, confirmDialog, h, promptDialog, toast } from './dom';
import { Explorer } from './explorer';
import { MonitorBar, MonitorSource } from './monitor';
import { MonitorDetail } from './taskmgr';
import { getSettings, onSettings } from './settings';
import { icon } from './icons';
import { TerminalView } from './terminal';
import { Titlebar } from './titlebar';
import { pref } from './util';

declare const __APP_VERSION__: string;

export interface ShellDeps {
  screen: HTMLElement;
  titlebar: Titlebar;
  onLock: () => void;
}

const MIN_SIDEBAR = 200;
const MAX_SIDEBAR = 560;

interface Tab {
  id: string;
  title: string;
  icon: string;
  view: HTMLElement;
  closable: boolean;
  /** Backend sessions owned by this tab (more than one after an SFTP reconnect); all closed with the tab. */
  sessions?: string[];
  state?: 'busy' | 'ok' | 'off';
  onShow?: () => void;
  dispose?: () => void;
  /** Unsaved editor changes inside the tab. */
  dirty?: () => boolean;
  /** Open a live copy of the terminal (same server, new shell, previous output shown above). */
  duplicate?: () => void | Promise<void>;
}

/** `user@host[:port]` -> parts, or null. */
export function parseQuick(s: string): { username: string; host: string; port: number } | null {
  const m = /^([^@\s:]+)@([^:\s]+)(?::(\d{1,5}))?$/.exec(s.trim());
  if (!m) return null;
  const port = m[3] ? Number(m[3]) : 22;
  if (port < 1 || port > 65535) return null;
  return { username: m[1], host: m[2], port };
}

export async function showShell({ screen, titlebar, onLock }: ShellDeps): Promise<void> {
  clear(screen);
  let servers: ServerView[] = [];
  let selected: string | null = null;
  const tabs: Tab[] = [];
  let activeId = 'home';
  let counter = 0;

  const list = h('ul', { class: 'servers', 'data-testid': 'server-list', role: 'listbox' });
  const search = h('input', { type: 'search', placeholder: 'Поиск сессий', 'data-testid': 'search', 'aria-label': 'Поиск сессий' }) as HTMLInputElement;
  const quick = h('input', { type: 'text', placeholder: 'user@host:порт — быстрое подключение', 'data-testid': 'quick', 'aria-label': 'Быстрое подключение', spellcheck: 'false' }) as HTMLInputElement;
  const home = h('div', { class: 'home', 'data-testid': 'home' });
  const work = h('main', { class: 'work' }, home);
  const statusCount = h('span', { 'data-testid': 'status-count' });
  const statusSessions = h('span', { 'data-testid': 'status-sessions' });

  // ---------- tabs ----------
  const homeTab: Tab = { id: 'home', title: 'Главная', icon: 'home', view: home, closable: false };
  tabs.push(homeTab);

  function renderTabs(): void {
    for (const t of tabs) t.view.style.display = t.id === activeId ? '' : 'none';
    titlebar.setTabs(
      tabs.map((t) => ({ id: t.id, title: t.title, icon: t.icon, closable: t.closable, state: t.state })),
      activeId,
      { onSelect: activate, onAdd: () => void add(), onClose: (id) => void closeTab(id), onRename: (id) => void renameTab(id) },
    );
    const n = tabs.filter((t) => t.state === 'ok').length;
    statusSessions.textContent = n ? `Подключено: ${n}` : '';
  }
  let onActivate: (id: string) => void = () => undefined;
  function activate(id: string): void {
    activeId = id;
    renderTabs();
    onActivate(id);
    tabs.find((t) => t.id === id)?.onShow?.();
  }
  function switchTab(delta: number): void {
    const i = tabs.findIndex((t) => t.id === activeId);
    activate(tabs[(i + delta + tabs.length) % tabs.length].id);
  }
  async function renameTab(id: string): Promise<void> {
    const t = tabs.find((x) => x.id === id);
    if (!t) return;
    const name = await promptDialog('Переименовать вкладку', 'Название', t.title, 'OK');
    if (name?.trim()) {
      t.title = name.trim();
      renderTabs();
    }
  }
  async function closeTab(id: string): Promise<void> {
    const t = tabs.find((x) => x.id === id);
    if (!t || !t.closable) return;
    const unsaved = t.dirty?.() ? ' В открытых файлах есть НЕСОХРАНЁННЫЕ изменения, они будут потеряны.' : '';
    if ((t.state !== 'off' || unsaved) && !(await confirmDialog('Закрыть сеанс?', `Соединение с «${t.title}» будет закрыто, запущенные в нём программы прервутся.${unsaved}`, 'Закрыть', true))) return;
    t.dispose?.();
    for (const s of t.sessions ?? []) void api.disconnect(s).catch(() => undefined);
    t.view.remove();
    tabs.splice(tabs.indexOf(t), 1);
    if (activeId === id) activeId = 'home';
    renderTabs();
  }

  // ---------- connecting ----------
  async function openSession(s: ConnectOpts, info: Connected, initialDir?: string | null): Promise<void> {
    const id = `s${++counter}`;
    const explorer = new Explorer(info.session_id, {
      onOpenFile: (path) => void openFile(path, false),
      onNewFile: (path) => void openFile(path, true),
      onReconnect: () => reconnect(),
    });
    const tab: Tab = { id, title: info.name, icon: 'terminal', closable: true, sessions: [info.session_id], state: 'busy', view: h('div', { class: 'session', 'data-testid': 'session' }) };
    const terminal = new TerminalView(info.session_id, s.serverId, {
      onClose: () => void closeTab(id),
      onState: (st) => {
        if (tab.state === st) return;
        tab.state = st;
        renderTabs();
      },
    });
    const panes = new PaneSet();
    // mini task manager under the terminal (numbers, small graphs); a click opens the detailed pane
    const mon = new MonitorSource(() => tab.sessions![tab.sessions!.length - 1], () => activeId === id);
    const bar = new MonitorBar(mon, () => openMonitor(), info.sftp_error ? 'Мониторинг недоступен: у сервера нет SFTP' : undefined);
    bar.setVisible(getSettings().monitor_enabled);
    const offSettings = onSettings((st) => bar.setVisible(st.monitor_enabled));
    const sessionNow = () => tab.sessions![tab.sessions!.length - 1];
    tab.onShow = () => panes.activate(panes.active || 'term');
    tab.dispose = () => {
      terminal.dispose();
      mon.stop();
      bar.dispose();
      offSettings();
      side.disposeAll();
    };
    tab.dirty = () => panes.anyDirty();
    // Duplicate: a second LIVE shell next to the first one (right column), started where the source is: same folder,
    // virtualenv and foreground program (found on the server, shown to the user before it is typed into the copy).
    const side = new PaneSet(true, () => setSide(false));
    let copies = 0;
    let source: TerminalView = terminal; // the terminal that was focused last: that one is duplicated
    terminal.el.addEventListener('focusin', () => (source = terminal));
    function setSide(on: boolean): void {
      sideCol.style.display = on ? '' : 'none';
      sideSplit.style.display = on ? '' : 'none';
    }
    tab.duplicate = async () => {
      const src = source;
      let plan: Awaited<ReturnType<typeof prepareClone>>;
      try {
        plan = await prepareClone(src);
      } catch (e) {
        // never fail silently: open the copy without commands and say why
        toast(`Копия без команд: ${e instanceof Error ? e.message : String(e)}`);
        plan = { prefill: src.snapshot(), startup: [], env: src.envCommands };
      }
      if (!plan) return;
      const hhmm = new Date().toLocaleTimeString('ru-RU', { hour: '2-digit', minute: '2-digit' });
      const note = `\r\n\x1b[2m──── копия терминала от ${hhmm}: ниже новая оболочка на том же сервере ────\x1b[0m\r\n`;
      const pid = `dup:${++copies}`;
      const copy: TerminalView = new TerminalView(
        sessionNow(),
        s.serverId,
        { onClose: () => void side.close(pid), onState: () => undefined },
        { prefill: plan.prefill ? plan.prefill + note : '', testid: copies === 1 ? 'terminal-copy' : `terminal-copy-${copies}`, startup: plan.startup, env: plan.env },
      );
      copy.el.addEventListener('focusin', () => (source = copy));
      side.add({
        id: pid,
        title: `Копия ${hhmm}`,
        view: h('div', { class: 'session-term' }, copy.el),
        closable: true,
        onShow: () => {
          copy.show();
          copy.term.focus();
        },
        dispose: () => {
          if (source === copy) source = terminal;
          copy.dispose();
        },
        beforeClose: () => confirmDialog('Закрыть копию терминала?', 'Соединение этой копии будет закрыто, запущенные в ней программы прервутся.', 'Закрыть', true),
      });
      setSide(true);
      requestAnimationFrame(() => void copy.start());
    };

    const exWrap = h('div', { class: 'session-ex' }, explorer.el);
    const savedW = Number(pref.get('explorer-width'));
    exWrap.style.width = `${savedW >= 240 && savedW <= 900 ? savedW : 360}px`;
    const split = h('div', { class: 'splitter', role: 'separator', 'aria-orientation': 'vertical', 'data-testid': 'ex-splitter' });
    split.addEventListener('pointerdown', (e) => {
      const x0 = e.clientX;
      const w0 = exWrap.getBoundingClientRect().width;
      split.setPointerCapture(e.pointerId);
      const move = (ev: PointerEvent) => (exWrap.style.width = `${Math.min(900, Math.max(240, w0 + ev.clientX - x0))}px`);
      const up = () => {
        split.removeEventListener('pointermove', move);
        split.removeEventListener('pointerup', up);
        pref.set('explorer-width', String(Math.round(exWrap.getBoundingClientRect().width)));
      };
      split.addEventListener('pointermove', move);
      split.addEventListener('pointerup', up);
    });
    panes.add({ id: 'term', title: 'Терминал', view: h('div', { class: 'session-term', 'data-testid': 'session-term' }, h('div', { class: 'term-col' }, terminal.el, bar.el)), closable: false, onShow: () => terminal.show() });
    // right column: copies of the terminal (hidden until the first copy), with a draggable divider
    const sideCol = h('div', { class: 'side-col', 'data-testid': 'side-col' }, side.el);
    const sideSplit = h('div', { class: 'splitter', role: 'separator', 'aria-orientation': 'vertical', 'data-testid': 'side-splitter' });
    sideSplit.addEventListener('pointerdown', (e) => {
      const x0 = e.clientX;
      const w0 = sideCol.getBoundingClientRect().width;
      sideSplit.setPointerCapture(e.pointerId);
      const move = (ev: PointerEvent) => (sideCol.style.flex = `0 0 ${Math.max(240, w0 - (ev.clientX - x0))}px`);
      const up = () => {
        sideSplit.removeEventListener('pointermove', move);
        sideSplit.removeEventListener('pointerup', up);
      };
      sideSplit.addEventListener('pointermove', move);
      sideSplit.addEventListener('pointerup', up);
    });
    setSide(false);
    tab.view.append(exWrap, split, panes.el, sideSplit, sideCol);
    work.append(tab.view);
    tabs.push(tab);
    activate(id);

    // The terminal is an independent ssh process: an SFTP reconnect only replaces the file panel's
    // session and keeps the old one alive for the running terminal.
    async function reconnect(): Promise<boolean> {
      const r = await connectFlow(s);
      if (!r) return false;
      tab.sessions!.push(r.session_id);
      explorer.setSession(r.session_id);
      if (!r.sftp_error) {
        bar.setUnavailable(undefined);
        mon.start();
      }
      if (r.sftp_error) {
        explorer.showUnavailable(r.sftp_error);
        return false;
      }
      await explorer.load(explorer.currentPath || undefined);
      return true;
    }

    function openMonitor(): void {
      if (panes.has('monitor')) return panes.activate('monitor');
      const detail = new MonitorDetail(mon);
      panes.add({ id: 'monitor', title: 'Монитор', view: detail.el, closable: true, onShow: () => mon.start(), dispose: () => detail.dispose() });
    }

    // ---- built-in editor: one pane per open file next to the terminal
    async function openFile(path: string, isNew: boolean): Promise<void> {
      const pid = `f:${path}`;
      if (panes.has(pid)) return panes.activate(pid);
      const ed = new EditorPane(path, {
        session: sessionNow,
        reconnect,
        onDirty: (d) => panes.setDirty(pid, d),
        onClose: () => void panes.close(pid),
      });
      let failure: unknown = null;
      try {
        await ed.load(isNew);
      } catch (e0) {
        failure = e0;
        // not decodable as text in the detected encoding: let the user name the encoding and try again
        if (e0 instanceof ApiError && e0.code === 'encoding') {
          const enc = await pickEncoding(e0.message);
          if (enc) {
            try {
              await ed.load(isNew, enc);
              failure = null;
            } catch (e2) {
              failure = e2;
            }
          }
        }
      }
      if (failure !== null) {
        const e = failure;
        ed.dispose();
        if (e instanceof ApiError && (e.code === 'binary' || e.code === 'too_large')) {
          if (await confirmDialog('Файл нельзя открыть в редакторе', `${e.message}

Скачать «${path.split('/').pop()}»?`, 'Скачать…')) await explorer.downloadPaths([path]);
        } else toast(e instanceof ApiError ? e.message : String(e), e instanceof ApiError ? e.code : undefined);
        return;
      }
      ed.el.addEventListener('saved', () => void explorer.load(explorer.currentPath || undefined));
      panes.add({
        id: pid,
        title: path.split('/').pop() ?? path,
        view: ed.el,
        closable: true,
        onShow: () => ed.focus(),
        dispose: () => ed.dispose(),
        beforeClose: async () => !ed.dirty || (await confirmDialog('Закрыть без сохранения?', `В «${path}» есть несохранённые изменения. Они будут потеряны.`, 'Закрыть', true)),
      });
      ed.focus();
    }

    // start the terminal after the first layout so that it opens with the right size
    requestAnimationFrame(() => void terminal.start());
    if (info.sftp_error) explorer.showUnavailable(info.sftp_error);
    else {
      mon.start();
      await explorer.load(initialDir || undefined);
    }
  }

  type ConnectOpts = Parameters<typeof connectFlow>[0];

  async function connectServer(s: ServerView): Promise<void> {
    const opts: ConnectOpts = { label: s.name, serverId: s.id };
    const info = await connectFlow(opts);
    if (info) await openSession(opts, info, s.initial_dir);
  }

  async function connectQuick(): Promise<void> {
    const q = parseQuick(quick.value);
    if (!q) return void toast('Введите адрес в формате user@host или user@host:порт.');
    const opts: ConnectOpts = { label: `${q.username}@${q.host}`, host: q.host, port: q.port, username: q.username };
    const info = await connectFlow(opts);
    if (info) {
      quick.value = '';
      await openSession(opts, info);
    }
  }
  quick.addEventListener('keydown', (e) => e.key === 'Enter' && void connectQuick());

  // ---------- server list ----------
  const reload = async () => {
    servers = await api.listServers();
    if (selected && !servers.some((s) => s.id === selected)) selected = null;
    renderList();
    renderHome();
    statusCount.textContent = `Сессий: ${servers.length}`;
  };
  const add = async () => (await serverForm()) && (await reload());
  const edit = async (s: ServerView) => (await serverForm(s)) && (await reload());
  const dup = async (s: ServerView) => {
    await api.duplicateServer(s.id);
    await reload();
  };
  const del = async (s: ServerView) => {
    if (!(await confirmDialog('Удалить сервер?', `«${s.name}» и сохранённый пароль будут удалены.`, 'Удалить', true))) return;
    await api.deleteServer(s.id);
    await reload();
  };
  const lock = async (silent = false) => {
    if (!silent && tabs.some((t) => t.sessions?.length) && !(await confirmDialog('Заблокировать хранилище?', 'Открытые сеансы будут закрыты, запущенные в них программы прервутся.', 'Заблокировать', true))) return;
    for (const t of tabs) {
      t.dispose?.();
      for (const s of t.sessions ?? []) void api.disconnect(s).catch(() => undefined);
    }
    await api.lock();
    onLock();
  };

  const iconBtn = (name: string, title: string, testid: string, fn: () => void) =>
    h('button', { class: 'icon-btn', title, 'aria-label': title, 'data-testid': testid, onclick: (e) => (e.stopPropagation(), fn()) }, icon(name, 14));

  function renderList(): void {
    clear(list);
    const q = search.value.trim().toLowerCase();
    const shown = servers.filter((s) => !q || s.name.toLowerCase().includes(q) || s.host.toLowerCase().includes(q));
    if (!shown.length) {
      list.append(h('li', { class: 'empty' }, servers.length ? 'Ничего не найдено.' : 'Сессий пока нет. Нажмите «Сессия» на панели.'));
      return;
    }
    for (const s of shown) {
      list.append(
        h(
          'li',
          {
            class: 'srv' + (s.id === selected ? ' selected' : ''),
            role: 'option',
            tabindex: '0',
            'data-testid': 'server-item',
            'data-id': s.id,
            onclick: () => {
              selected = s.id;
              renderList();
            },
            ondblclick: () => void connectServer(s),
            onkeydown: (e) => {
              const k = (e as KeyboardEvent).key;
              if (k === 'Enter') void connectServer(s);
              if (k === 'F2') void edit(s);
              if (k === 'Delete') void del(s);
            },
          },
          h('span', { class: 'srv-ico' }, icon('terminal', 16)),
          h('div', { class: 'grow' }, h('div', { class: 'srv-name' }, s.name), h('div', { class: 'sub' }, `${s.username}@${s.host}:${s.port}${s.key_path ? " · ключ" : ""}`)),
          h(
            'div',
            { class: 'srv-actions' },
            iconBtn('plug', 'Подключиться (Enter)', 'connect', () => void connectServer(s)),
            iconBtn('edit', 'Изменить (F2)', 'edit', () => void edit(s)),
            iconBtn('copy', 'Дублировать', 'dup', () => void dup(s)),
            iconBtn('trash', 'Удалить (Del)', 'delete', () => void del(s)),
          ),
        ),
      );
    }
  }

  function renderHome(): void {
    clear(home);
    home.append(
      h(
        'div',
        { class: 'home-head' },
        h('h1', {}, 'Добро пожаловать в d256 Shell'),
        h('p', {}, 'Портативный SSH-клиент: терминал, проводник файлов и редактор в одном окне.'),
        h('div', { class: 'row' }, h('button', { class: 'primary', 'data-testid': 'home-add', onclick: () => void add() }, icon('plus', 14), 'Новая сессия')),
      ),
      h('h2', { class: 'section' }, 'Сессии'),
    );
    if (!servers.length) {
      home.append(h('div', { class: 'home-empty' }, 'Здесь появятся ваши серверы. Добавьте первый — пароль будет храниться в зашифрованном хранилище.'));
      return;
    }
    const grid = h('div', { class: 'cards' });
    for (const s of servers) {
      grid.append(
        h(
          'button',
          { class: 'card-srv', 'data-testid': 'home-card', ondblclick: () => void connectServer(s), onclick: () => ((selected = s.id), renderList()) },
          h('span', { class: 'srv-ico big' }, icon('terminal', 20)),
          h('span', { class: 'grow left' }, h('strong', {}, s.name), h('span', { class: 'sub' }, `${s.username}@${s.host}:${s.port}${s.key_path ? " · ключ" : ""}`)),
        ),
      );
    }
    home.append(grid);
  }

  search.addEventListener('input', renderList);

  // ---------- toolbar (MobaXterm-style big buttons) ----------
  const sidebar = h('aside', { class: 'sidebar', 'data-testid': 'sidebar' });
  const tbtn = (name: string, label: string, testid: string, fn: () => void) =>
    h('button', { class: 'tool', 'data-testid': testid, onclick: fn }, icon(name, 20), h('span', {}, label));
  /** Copy of the active terminal: a new live shell that starts with the original screen and scrollback. */
  const dupActive = () => {
    const t = tabs.find((x) => x.id === activeId);
    if (t?.duplicate) t.duplicate();
    else toast('Откройте вкладку SSH: дублируется её терминал.');
  };
  const toggleSidebar = () => {
    const hidden = sidebar.classList.toggle('hidden');
    splitter.classList.toggle('hidden', hidden);
    if (activeId === 'home') pref.set('sidebar-hidden', hidden ? '1' : '0');
  };
  const toolbar = h(
    'div',
    { class: 'toolbar', role: 'toolbar' },
    tbtn('plus', 'Сессия', 'add', () => void add()),
    tbtn('plug', 'Подключить', 'toolbar-connect', () => {
      const s = servers.find((x) => x.id === selected);
      if (s) void connectServer(s);
      else toast('Выберите сессию в списке слева.');
    }),
    tbtn('sidebar', 'Панель', 'toggle-sidebar', toggleSidebar),
    tbtn('copy', 'Дублировать', 'duplicate-term', () => dupActive()),
    h('div', { class: 'tool-sep' }),
    tbtn('key', 'Мастер-пароль', 'change-master', () => void changeMasterDialog()),
    tbtn('settings', 'Настройки', 'settings', () => void settingsDialog()),
    h('div', { class: 'grow' }),
    tbtn('lock', 'Заблокировать', 'lock', () => void lock()),
  );

  // ---------- left rail + sidebar + splitter ----------
  const rail = h(
    'nav',
    { class: 'rail', 'aria-label': 'Разделы' },
    h('button', { class: 'rail-btn active', title: 'Сессии', 'data-testid': 'rail-sessions' }, icon('server', 20)),
  );
  sidebar.append(
    h('div', { class: 'side-head' }, h('span', {}, 'Сессии')),
    h('div', { class: 'search' }, icon('plug', 14), quick),
    h('div', { class: 'search' }, icon('search', 14), search),
    list,
  );
  const saved = Number(pref.get('sidebar-width'));
  sidebar.style.width = `${saved >= MIN_SIDEBAR && saved <= MAX_SIDEBAR ? saved : 280}px`;

  const splitter = h('div', { class: 'splitter', role: 'separator', 'aria-orientation': 'vertical', 'data-testid': 'splitter' });
  splitter.addEventListener('pointerdown', (e) => {
    const start = e.clientX;
    const w0 = sidebar.getBoundingClientRect().width;
    splitter.setPointerCapture(e.pointerId);
    const move = (ev: PointerEvent) => {
      sidebar.style.width = `${Math.min(MAX_SIDEBAR, Math.max(MIN_SIDEBAR, w0 + ev.clientX - start))}px`;
    };
    const up = () => {
      splitter.removeEventListener('pointermove', move);
      splitter.removeEventListener('pointerup', up);
      pref.set('sidebar-width', String(Math.round(sidebar.getBoundingClientRect().width)));
    };
    splitter.addEventListener('pointermove', move);
    splitter.addEventListener('pointerup', up);
  });
  if (pref.get('sidebar-hidden') === '1') {
    sidebar.classList.add('hidden');
    splitter.classList.add('hidden');
  }

  // The session list is only needed on the home tab: while working in an SSH tab it is hidden to give
  // the terminal room (the toolbar button / Ctrl+Shift+B still toggles it). The saved preference applies to home.
  const setSidebar = (hidden: boolean) => {
    sidebar.classList.toggle('hidden', hidden);
    splitter.classList.toggle('hidden', hidden);
  };
  onActivate = (id) => setSidebar(id === 'home' ? pref.get('sidebar-hidden') === '1' : true);
  // ---------- status bar ----------
  const status = h(
    'footer',
    { class: 'statusbar' },
    h('span', { class: 'st' }, icon('unlock', 12), 'Хранилище разблокировано'),
    statusCount,
    statusSessions,
    h('div', { class: 'grow' }),
    h('span', {}, `d256 Shell ${__APP_VERSION__}`),
  );

  screen.append(toolbar, h('div', { class: 'body' }, rail, sidebar, splitter, work), status);
  renderTabs();

  // Keyboard shortcuts. Ctrl+T / Ctrl+W / Ctrl+B are also readline/tmux keys, so while the terminal has
  // focus they go to the server; the Ctrl+Shift variants (like Windows Terminal) work everywhere.
  const onKey = (e: KeyboardEvent) => {
    if (!e.ctrlKey || e.altKey) return;
    const inTerminal = !!(document.activeElement as HTMLElement | null)?.closest('.xterm');
    const k = e.key.toLowerCase();
    if (k === 'tab') {
      e.preventDefault();
      switchTab(e.shiftKey ? -1 : 1);
      return;
    }
    if (!e.shiftKey && inTerminal) return;
    if (k === 'b') (e.preventDefault(), toggleSidebar());
    if (k === 't') (e.preventDefault(), void add());
    if (k === 'w' && activeId !== 'home') (e.preventDefault(), void closeTab(activeId));
    if (k === 'd') (e.preventDefault(), dupActive());
  };
  document.addEventListener('keydown', onKey);
  new MutationObserver((_, obs) => {
    if (!document.body.contains(toolbar)) {
      document.removeEventListener('keydown', onKey);
      obs.disconnect();
    }
  }).observe(screen, { childList: true });

  // Closing the window with unsaved editor changes asks first.
  const anyDirty = () => tabs.some((t) => t.dirty?.());
  const beforeUnload = (e: BeforeUnloadEvent) => {
    if (anyDirty()) e.preventDefault();
  };
  window.addEventListener('beforeunload', beforeUnload);

  // Auto-lock after N idle minutes (settings; 0 = off). Never while there are unsaved editor changes.
  let lastActive = Date.now();
  const bump = () => (lastActive = Date.now());
  const activity = ['pointerdown', 'keydown', 'wheel', 'pointermove'];
  for (const ev of activity) document.addEventListener(ev, bump, true);
  const idleTimer = window.setInterval(() => {
    const m = getSettings().auto_lock_minutes;
    if (m > 0 && Date.now() - lastActive > m * 60_000 && !anyDirty()) void lock(true);
  }, 10_000);
  new MutationObserver((_, obs) => {
    if (!document.body.contains(toolbar)) {
      clearInterval(idleTimer);
      for (const ev of activity) document.removeEventListener(ev, bump, true);
      obs.disconnect();
    }
  }).observe(screen, { childList: true });
  if (inTauri) {
    void getCurrentWindow().onCloseRequested(async (ev) => {
      if (!anyDirty()) return;
      ev.preventDefault();
      if (await confirmDialog('Закрыть d256 Shell?', 'В открытых файлах есть несохранённые изменения. Они будут потеряны.', 'Закрыть', true)) await getCurrentWindow().destroy();
    });
  }

  await reload();
}
