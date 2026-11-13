// Main window layout (MobaXterm-like): toolbar, left rail + sessions sidebar, tabbed work area, status bar.
import { EditorPane, pickEncoding } from './editor';
import { prepareClone } from './clone';
import { PaneSet } from './panes';
import { api, ApiError, Connected, inTauri, ScriptItem, ServerView, ShellProfile } from './api';
import { iconFor, iconKey, pickIcon } from './iconkit';
import { NotePane, ScriptPane } from './localedit';
import { createNotes, createScripts, createTerminals, Section } from './sections';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { connectFlow } from './connect';
import { changeMasterDialog, serverForm, settingsDialog } from './dialogs';
import { clear, confirmDialog, h, promptDialog, toast } from './dom';
import { Explorer } from './explorer';
import { LOCAL_TARGET, MonitorBar, monitors } from './monitor';
import { MonitorColumn } from './moncol';
import { MonitorDetail } from './taskmgr';
import { getSettings, onSettings } from './settings';
import { autoCheck, dismissUpdate, openRelease } from './updates';
import { icon } from './icons';
import { TerminalView } from './terminal';
import { Titlebar } from './titlebar';
import { pref } from './util';
import { t, locale } from './i18n';

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
  iconKey?: string;
  view: HTMLElement;
  closable: boolean;
  /** Backend sessions owned by this tab (more than one after an SFTP reconnect); all closed with the tab. */
  sessions?: string[];
  state?: 'busy' | 'ok' | 'off';
  onShow?: () => void;
  dispose?: () => void;
  /** Unsaved editor changes inside the tab. */
  dirty?: () => boolean;
  /** A local shell, script or note (no remote session: closing needs no connection warning). */
  local?: boolean;
  /** Script id of a run tab (a click on the script switches to it while it runs). */
  runKey?: string;
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
  type SectionName = 'sessions' | 'terminals' | 'scripts' | 'notes';
  let section: SectionName = 'sessions';
  let activeId = 'home';
  let counter = 0;

  const list = h('ul', { class: 'servers', 'data-testid': 'server-list', role: 'listbox' });
  const search = h('input', { type: 'search', placeholder: t('Search sessions'), 'data-testid': 'search', 'aria-label': t('Search sessions') }) as HTMLInputElement;
  const quick = h('input', { type: 'text', placeholder: t('user@host:port: quick connect'), 'data-testid': 'quick', 'aria-label': t('Quick connect'), spellcheck: 'false' }) as HTMLInputElement;
  const home = h('div', { class: 'home', 'data-testid': 'home' });
  const work = h('main', { class: 'work' }, home);
  const statusCount = h('span', { 'data-testid': 'status-count' });
  const statusSessions = h('span', { 'data-testid': 'status-sessions' });
  const statusUpdate = h('span', { class: 'st-update', 'data-testid': 'update-banner' });
  statusUpdate.style.display = 'none';

  // ---------- tabs ----------
  const homeTab: Tab = { id: 'home', title: t('Home'), icon: 'home', view: home, closable: false };
  tabs.push(homeTab);

  function renderTabs(): void {
    for (const t of tabs) t.view.style.display = t.id === activeId ? '' : 'none';
    titlebar.setTabs(
      tabs.map((t) => ({ id: t.id, title: t.title, icon: t.icon, iconKey: t.iconKey, closable: t.closable, state: t.state })),
      activeId,
      { onSelect: activate, onAdd: () => void add(), onClose: (id) => void closeTab(id), onRename: (id) => void renameTab(id) },
    );
    renderOpenTabs();
    const n = tabs.filter((t) => t.state === 'ok').length;
    statusSessions.textContent = n ? t('Connected: {0}', n) : '';
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
    const tab = tabs.find((x) => x.id === id);
    if (!tab) return;
    const name = await promptDialog(t('Rename tab'), t('Name'), tab.title, 'OK');
    if (name?.trim()) {
      tab.title = name.trim();
      renderTabs();
    }
  }
  async function closeTab(id: string): Promise<void> {
    const tab = tabs.find((x) => x.id === id);
    if (!tab || !tab.closable) return;
    const unsaved = tab.dirty?.() ? ' ' + t('There are UNSAVED changes in open files, they will be lost.') : '';
    if (((tab.state ?? 'off') !== 'off' || unsaved) && !(await confirmDialog(tab.local ? t('Close the tab?') : t('Close the session?'), tab.local ? t('The programs running in “{0}” will be interrupted.{1}', tab.title, unsaved) : t('Connection to “{0}” will be closed and the programs running in it will be interrupted.{1}', tab.title, unsaved), t('Close'), true))) return;
    tab.dispose?.();
    for (const s of tab.sessions ?? []) void api.disconnect(s).catch(() => undefined);
    tab.view.remove();
    tabs.splice(tabs.indexOf(tab), 1);
    if (activeId === id) activate('home');
    else renderTabs();
  }

  /** The monitor column to the right of a terminal (the ticked indicators of the chosen target), folded by the button of
   * the bar and resized by its splitter; the choice is remembered. */
  function monitorColumn(own: string, active: () => boolean): { el: HTMLElement; split: HTMLElement; toggle: () => void; dispose: () => void } {
    // the details exist only while the column is shown (a folded column draws nothing and has no elements)
    let detail: MonitorColumn | null = null;
    const el = h('div', { class: 'mon-col', 'data-testid': 'mon-col' });
    const w = Number(pref.get('mon-col-width'));
    el.style.width = `${w >= 260 && w <= 900 ? w : 340}px`;
    const split = h('div', { class: 'splitter mon-col-split', role: 'separator', 'aria-orientation': 'vertical', 'data-testid': 'mon-col-splitter' });
    const show = (on: boolean) => {
      el.classList.toggle('hidden', !on);
      split.classList.toggle('hidden', !on);
      pref.set('mon-col', on ? '1' : '0');
      if (on && !detail) {
        detail = new MonitorColumn(own, active);
        el.append(detail.el);
      } else if (!on && detail) {
        detail.dispose();
        detail.el.remove();
        detail = null;
      }
    };
    show(pref.get('mon-col') === '1');
    split.addEventListener('pointerdown', (e) => {
      const x0 = e.clientX;
      const w0 = el.getBoundingClientRect().width;
      split.setPointerCapture(e.pointerId);
      const move = (ev: PointerEvent) => (el.style.width = `${Math.min(900, Math.max(260, w0 - (ev.clientX - x0)))}px`);
      const up = () => {
        split.removeEventListener('pointermove', move);
        split.removeEventListener('pointerup', up);
        pref.set('mon-col-width', String(Math.round(el.getBoundingClientRect().width)));
      };
      split.addEventListener('pointermove', move);
      split.addEventListener('pointerup', up);
    });
    return { el, split, toggle: () => show(el.classList.contains('hidden')), dispose: () => detail?.dispose() };
  }

  // ---------- local tabs: shells, scripts, notes ----------
  let localCounter = 0;
  const dropTab = (tb: Tab) => {
    tb.dispose?.();
    tb.view.remove();
    tabs.splice(tabs.indexOf(tb), 1);
    if (activeId === tb.id) activate('home');
    else renderTabs();
  };
  const sectionCtx = {
    openTerminal: (p: ShellProfile) => openTerminal(p),
    openScript: (it: ScriptItem) => void openScriptTab(it),
    runScript: (it: ScriptItem, copy = false) => openRunTab(it, copy),
    retitle: (sid: string) => retitleScript(sid),
    openNote: (id?: string) => void openNoteTab(id),
    closeTabs: (prefix: string) => {
      for (const tb of [...tabs]) if (tb.id.startsWith(prefix)) dropTab(tb);
    },
    refreshHome: () => renderHome(),
  };
  const sections: Record<'terminals' | 'scripts' | 'notes', Section> = {
    terminals: createTerminals(sectionCtx),
    scripts: createScripts(sectionCtx),
    notes: createNotes(sectionCtx),
  };

  function addLocalTab(tab: Tab): void {
    tab.local = true;
    work.append(tab.view);
    tabs.push(tab);
    activate(tab.id);
  }

  function localTerminal(title: string, key: string | undefined, local: { profile?: string; script?: string }, testid: string, runKey?: string): void {
    const id = `local${++localCounter}`;
    const tab: Tab = { id, title, icon: 'terminal', iconKey: key, closable: true, state: 'busy', runKey, view: h('div', { class: 'session', 'data-testid': testid }) };
    const term = new TerminalView('', undefined, {
      onClose: () => void closeTab(id),
      onState: (st) => {
        if (tab.state === st) return;
        tab.state = st;
        renderTabs();
      },
    }, { testid: 'local-terminal', local });
    // the explorer of THIS computer on the left, like the SFTP explorer of an SSH tab
    const panes = new PaneSet();
    const explorer = new Explorer('local', {
      onOpenFile: (path) => void openLocalFile(path),
      onNewFile: (path) => void createLocalFile(path),
    }, true);
    const exWrap = h('div', { class: 'session-ex', 'data-testid': 'local-ex' }, explorer.el);
    const savedW = Number(pref.get('explorer-width'));
    exWrap.style.width = `${savedW >= 240 && savedW <= 900 ? savedW : 360}px`;
    const split = h('div', { class: 'splitter', role: 'separator', 'aria-orientation': 'vertical', 'data-testid': 'local-ex-splitter' });
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
    // load of THIS computer (processor, memory, disks, network, video card): a bar under the terminal and a column on the right
    const watch = monitors.watch(LOCAL_TARGET, () => activeId === id);
    const mon = watch.src;
    const mcol = monitorColumn(LOCAL_TARGET, () => activeId === id);
    const bar = new MonitorBar(mon, () => openMonitor(), undefined, () => mcol.toggle());
    bar.setVisible(getSettings().monitor_enabled);
    const offSettings = onSettings((st) => bar.setVisible(st.monitor_enabled));
    function openMonitor(): void {
      if (panes.has('monitor')) return panes.activate('monitor');
      const detail = new MonitorDetail(mon);
      panes.add({ id: 'monitor', title: t('Monitor'), view: detail.el, closable: true, onShow: () => mon.start(), dispose: () => detail.dispose() });
    }
    panes.add({ id: 'term', title: t('Terminal'), view: h('div', { class: 'session-term' }, h('div', { class: 'term-col' }, term.el, bar.el)), closable: false, onShow: () => term.show() });
    tab.view.append(exWrap, split, panes.el, mcol.split, mcol.el);
    tab.onShow = () => panes.activate(panes.active || 'term');
    tab.dispose = () => {
      term.dispose();
      watch.release();
      bar.dispose();
      mcol.dispose();
      offSettings();
      panes.disposeAll();
    };
    tab.dirty = () => panes.anyDirty();
    addLocalTab(tab);
    requestAnimationFrame(() => void term.start());
    void explorer.load();

    /** A file chosen in the local explorer opens in the built-in editor (same codec as the remote editor). */
    async function openLocalFile(path: string): Promise<void> {
      const pid = 'f:' + path;
      if (panes.has(pid)) return panes.activate(pid);
      const name = path.slice(path.lastIndexOf('/') + 1);
      const item: ScriptItem = { id: 'file:' + path, name, ext: name.includes('.') ? name.slice(name.lastIndexOf('.') + 1).toLowerCase() : '', kind: 'file', path, exists: true };
      const ed = new ScriptPane(item, { onDirty: (d) => panes.setDirty(pid, d), onClose: () => void panes.close(pid) }, () => undefined);
      if (!(await ed.open())) return;
      panes.add({
        id: pid,
        title: name,
        view: ed.el,
        closable: true,
        onShow: () => ed.focus(),
        beforeClose: async () => !ed.dirty || (await confirmDialog(t('Close without saving?'), t('“{0}” has unsaved changes. They will be lost.', path), t('Close'), true)),
      });
      ed.focus();
    }

    async function createLocalFile(path: string): Promise<void> {
      const i = path.lastIndexOf('/');
      try {
        await api.lfs('touch', { parent: path.slice(0, i) || '/', name: path.slice(i + 1) });
      } catch (e) {
        return void toast(e instanceof ApiError ? e.message : String(e), e instanceof ApiError ? e.code : undefined);
      }
      await openLocalFile(path);
    }
  }

  function openTerminal(p: ShellProfile): void {
    localTerminal(p.name, iconKey('profile', p.id), { profile: p.id }, 'local-tab');
  }
  const scriptTitle = (it: ScriptItem) => getSettings().script_names[it.id] || it.name;
  function openRunTab(it: ScriptItem, copy = false): void {
    scriptCache.set(it.id, it);
    if (!copy) {
      const live = tabs.find((x) => x.runKey === it.id && x.state !== 'off');
      if (live) return activate(live.id); // running: show it
      const ended = tabs.find((x) => x.runKey === it.id);
      if (ended) dropTab(ended); // finished: the new run replaces it
    }
    localTerminal(scriptTitle(it), iconKey('script', it.id), { script: it.id }, 'run-tab', it.id);
  }
  function retitleScript(sid: string): void {
    for (const tb of tabs) {
      if (tb.runKey === sid || tb.id === 'script:' + sid) {
        const sc = scriptCache.get(sid);
        if (sc) tb.title = tb.dirty?.() ? scriptTitle(sc) + ' ●' : scriptTitle(sc);
      }
    }
    renderTabs();
  }
  const scriptCache = new Map<string, ScriptItem>();

  async function openScriptTab(it: ScriptItem): Promise<void> {
    const id = 'script:' + it.id;
    scriptCache.set(it.id, it);
    if (tabs.some((x) => x.id === id)) return activate(id);
    const tab: Tab = { id, title: scriptTitle(it), icon: 'script', iconKey: iconKey('script', it.id), closable: true, view: h('div', { class: 'session editor-tab', 'data-testid': 'script-tab' }) };
    const pane = new ScriptPane(
      it,
      {
        onDirty: (d) => {
          tab.title = d ? scriptTitle(it) + ' ●' : scriptTitle(it);
          renderTabs();
        },
        onClose: () => void closeTab(id),
      },
      () => openRunTab(it),
    );
    if (!(await pane.open())) return;
    tab.view.append(pane.el);
    tab.dirty = () => pane.dirty;
    tab.onShow = () => pane.focus();
    addLocalTab(tab);
    pane.focus();
  }

  async function openNoteTab(noteId?: string): Promise<void> {
    if (noteId && tabs.some((x) => x.id === 'note:' + noteId)) return activate('note:' + noteId);
    let tabId = noteId ? 'note:' + noteId : `note:new${++localCounter}`;
    const tab: Tab = { id: tabId, title: t('Untitled'), icon: 'note', iconKey: noteId ? iconKey('note', noteId) : undefined, closable: true, view: h('div', { class: 'session editor-tab', 'data-testid': 'note-tab' }) };
    const pane = new NotePane(
      noteId,
      {
        onDirty: () => undefined,
        onClose: () => void closeTab(tabId),
        onTitle: (title, nid) => {
          tab.title = title;
          if (tab.id !== 'note:' + nid) {
            tabId = 'note:' + nid;
            tab.id = tabId;
            tab.iconKey = iconKey('note', nid);
            activeId = tabId;
          }
          renderTabs();
          void sections.notes.refresh().then(() => section === 'notes' && renderHome());
        },
      },
      async () => {
        if (!pane.id) return void dropTab(tab);
        if (!(await confirmDialog(t('Delete note?'), t('“{0}” will be deleted from the vault.', tab.title), t('Delete'), true))) return;
        await api.noteDelete(pane.id);
        dropTab(tab);
        await sections.notes.refresh();
        renderHome();
      },
    );
    try {
      await pane.load();
    } catch (e) {
      toast(e instanceof ApiError ? e.message : String(e), e instanceof ApiError ? e.code : undefined);
      return;
    }
    if (noteId) tab.title = (await api.noteGet(noteId)).title;
    tab.view.append(pane.el);
    tab.dispose = () => {
      void pane.flush().finally(() => pane.dispose());
    };
    tab.onShow = () => pane.focus();
    addLocalTab(tab);
    pane.focus();
  }

  // ---------- tabs on the left (Settings: Tabs) ----------
  function applyTabsPosition(): void {
    const left = getSettings().tabs_position === 'left';
    titlebar.setVertical(left);
    sidebar.classList.toggle('with-tabs', left);
    renderOpenTabs();
    if (left && sidebar.classList.contains('hidden')) toggleSidebar();
  }
  function renderOpenTabs(): void {
    clear(openTabsEl);
    if (getSettings().tabs_position !== 'left') return;
    openTabsEl.append(h('div', { class: 'side-head' }, h('span', {}, t('Open tabs'))));
    for (const tb of tabs) {
      openTabsEl.append(
        h(
          'div',
          { class: 'vtab' + (tb.id === activeId ? ' active' : ''), role: 'tab', tabindex: '0', title: tb.title, 'data-testid': `vtab-${tb.id}`, onclick: () => activate(tb.id), onkeydown: (e) => (e as KeyboardEvent).key === 'Enter' && activate(tb.id) },
          tb.state ? h('span', { class: `dot ${tb.state}` }) : null,
          tb.iconKey ? iconFor(tb.iconKey, tb.icon, 14) : icon(tb.icon, 14),
          h('span', { class: 'grow vtab-title' }, tb.title),
          tb.closable ? h('button', { class: 'icon-btn', title: t('Close tab'), 'aria-label': t('Close tab'), 'data-testid': `vtab-close-${tb.id}`, onclick: (e) => (e.stopPropagation(), void closeTab(tb.id)) }, icon('close', 12)) : null,
        ),
      );
    }
  }

  // ---------- connecting ----------
  async function openSession(s: ConnectOpts, info: Connected, initialDir?: string | null): Promise<void> {
    const id = `s${++counter}`;
    const explorer = new Explorer(info.session_id, {
      onOpenFile: (path) => void openFile(path, false),
      onNewFile: (path) => void openFile(path, true),
      onReconnect: () => reconnect(),
    });
    const tab: Tab = { id, title: info.name, icon: 'terminal', iconKey: s.serverId ? iconKey('server', s.serverId) : undefined, closable: true, sessions: [info.session_id], state: 'busy', view: h('div', { class: 'session', 'data-testid': 'session' }) };
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
    // one monitor per server, shared by all its tabs; a quick connection has none to share
    const target = s.serverId ? `server:${s.serverId}` : `adhoc:${id}`;
    const sessionNow = () => tab.sessions![tab.sessions!.length - 1];
    let unregister: (() => void) | null = info.sftp_error ? null : monitors.register(target, info.name, sessionNow);
    const watch = monitors.watch(target, () => activeId === id);
    const mon = watch.src;
    const mcol = monitorColumn(target, () => activeId === id);
    const bar = new MonitorBar(mon, () => openMonitor(), info.sftp_error ? t('Monitoring is unavailable: the server has no SFTP') : undefined, () => mcol.toggle());
    bar.setVisible(getSettings().monitor_enabled);
    const offSettings = onSettings((st) => bar.setVisible(st.monitor_enabled));
    tab.onShow = () => panes.activate(panes.active || 'term');
    tab.dispose = () => {
      terminal.dispose();
      watch.release();
      unregister?.();
      bar.dispose();
      mcol.dispose();
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
        toast(t('Copy without commands: {0}', e instanceof Error ? e.message : String(e)));
        plan = { prefill: src.snapshot(), startup: [], env: src.envCommands };
      }
      if (!plan) return;
      const hhmm = new Date().toLocaleTimeString(locale(), { hour: '2-digit', minute: '2-digit' });
      const note = `\r\n\x1b[2m──── ${t('copy of the terminal from {0}: below is a new shell on the same server', hhmm)} ────\x1b[0m\r\n`;
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
        title: t('Copy {0}', hhmm),
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
        beforeClose: () => confirmDialog(t('Close the terminal copy?'), t('The connection of this copy will be closed and the programs running in it will be interrupted.'), t('Close'), true),
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
    panes.add({ id: 'term', title: t('Terminal'), view: h('div', { class: 'session-term', 'data-testid': 'session-term' }, h('div', { class: 'term-col' }, terminal.el, bar.el)), closable: false, onShow: () => terminal.show() });
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
    tab.view.append(exWrap, split, panes.el, sideSplit, sideCol, mcol.split, mcol.el);
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
        unregister ??= monitors.register(target, info.name, sessionNow);
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
      panes.add({ id: 'monitor', title: t('Monitor'), view: detail.el, closable: true, onShow: () => mon.start(), dispose: () => detail.dispose() });
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
          if (await confirmDialog(t('The file cannot be opened in the editor'), t('{0}\n\nDownload “{1}”?', e.message, path.split('/').pop()), t('Download…'))) await explorer.downloadPaths([path]);
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
        beforeClose: async () => !ed.dirty || (await confirmDialog(t('Close without saving?'), t('“{0}” has unsaved changes. They will be lost.', path), t('Close'), true)),
      });
      ed.focus();
    }

    // start the terminal after the first layout so that it opens with the right size
    requestAnimationFrame(() => void terminal.start());
    if (info.sftp_error) explorer.showUnavailable(info.sftp_error);
    else {
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
    if (!q) return void toast(t('Enter the address as user@host or user@host:port.'));
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
    statusCount.textContent = t('Sessions: {0}', servers.length);
  };
  const add = async () => (await serverForm()) && (await reload());
  const edit = async (s: ServerView) => (await serverForm(s)) && (await reload());
  const dup = async (s: ServerView) => {
    await api.duplicateServer(s.id);
    await reload();
  };
  const del = async (s: ServerView) => {
    if (!(await confirmDialog(t('Delete server?'), t('“{0}” and the saved password will be deleted.', s.name), t('Delete'), true))) return;
    await api.deleteServer(s.id);
    await reload();
  };
  const lock = async (silent = false) => {
    if (!silent && tabs.some((t) => t.sessions?.length) && !(await confirmDialog(t('Lock the vault?'), t('The open sessions will be closed and the programs running in them will be interrupted.'), t('Lock'), true))) return;
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
      list.append(h('li', { class: 'empty' }, servers.length ? t('Nothing found.') : t('No sessions yet. Click Session on the toolbar.')));
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
          h('span', { class: 'srv-ico' }, iconFor(iconKey('server', s.id), 'terminal', 16)),
          h('div', { class: 'grow' }, h('div', { class: 'srv-name' }, s.name), h('div', { class: 'sub' }, `${s.username}@${s.host}:${s.port}${s.key_path ? " " + t("· key") : ""}`)),
          h(
            'div',
            { class: 'srv-actions' },
            iconBtn('plug', t('Connect (Enter)'), 'connect', () => void connectServer(s)),
            iconBtn('edit', t('Edit (F2)'), 'edit', () => void edit(s)),
            iconBtn('copy', t('Duplicate'), 'dup', () => void dup(s)),
            iconBtn('image', t('Change icon…'), 'server-icon', () => void pickIcon(iconKey('server', s.id), 'terminal').then((c) => c && (renderList(), renderHome()))),
            iconBtn('trash', t('Delete (Del)'), 'delete', () => void del(s)),
          ),
        ),
      );
    }
  }

  function renderHome(): void {
    clear(home);
    if (section !== 'sessions') {
      home.append(sections[section].home());
      return;
    }
    home.append(
      h(
        'div',
        { class: 'home-head' },
        h('h1', {}, t('Welcome to d256 Shell')),
        h('p', {}, t('Portable SSH client: terminal, file explorer and editor in one window.')),
        h('div', { class: 'row' }, h('button', { class: 'primary', 'data-testid': 'home-add', onclick: () => void add() }, icon('plus', 14), t('New session'))),
      ),
      h('h2', { class: 'section' }, t('Sessions')),
    );
    if (!servers.length) {
      home.append(h('div', { class: 'home-empty' }, t('Your servers will appear here. Add the first one: its password will be kept in the encrypted vault.')));
      return;
    }
    const grid = h('div', { class: 'cards' });
    for (const s of servers) {
      grid.append(
        h(
          'button',
          { class: 'card-srv', 'data-testid': 'home-card', ondblclick: () => void connectServer(s), onclick: () => ((selected = s.id), renderList()) },
          h('span', { class: 'srv-ico big' }, iconFor(iconKey('server', s.id), 'terminal', 20)),
          h('span', { class: 'grow left' }, h('strong', {}, s.name), h('span', { class: 'sub' }, `${s.username}@${s.host}:${s.port}${s.key_path ? " " + t("· key") : ""}`)),
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
    const tab = tabs.find((x) => x.id === activeId);
    if (tab?.duplicate) tab.duplicate();
    else toast(t('Open an SSH tab: its terminal is duplicated.'));
  };
  const toggleSidebar = () => {
    const hidden = sidebar.classList.toggle('hidden');
    splitter.classList.toggle('hidden', hidden);
    if (activeId === 'home') pref.set('sidebar-hidden', hidden ? '1' : '0');
  };
  const toolbar = h(
    'div',
    { class: 'toolbar', role: 'toolbar' },
    tbtn('plus', t('Session'), 'add', () => void add()),
    tbtn('plug', t('Connect'), 'toolbar-connect', () => {
      const s = servers.find((x) => x.id === selected);
      if (s) void connectServer(s);
      else toast(t('Select a session in the list on the left.'));
    }),
    tbtn('sidebar', t('Panel'), 'toggle-sidebar', toggleSidebar),
    tbtn('copy', t('Duplicate'), 'duplicate-term', () => dupActive()),
    h('div', { class: 'tool-sep' }),
    tbtn('key', t('Master password'), 'change-master', () => void changeMasterDialog()),
    tbtn('settings', t('Settings'), 'settings', () => void settingsDialog()),
    h('div', { class: 'grow' }),
    tbtn('lock', t('Lock'), 'lock', () => void lock()),
  );

  // ---------- left rail + sidebar + splitter ----------
  const railBtns = new Map<SectionName, HTMLElement>();
  const rail = h('nav', { class: 'rail', 'aria-label': t('Sections') });
  const railBtn = (name: SectionName, title: string, glyph: string) => {
    const b = h('button', { class: 'rail-btn', title, 'aria-label': title, 'data-testid': `rail-${name}`, onclick: () => (name === section ? toggleSidebar() : showSection(name)) }, icon(glyph, 20));
    railBtns.set(name, b);
    rail.append(b);
  };
  railBtn('sessions', t('Sessions'), 'server');
  railBtn('terminals', t('Terminals'), 'terminal');
  railBtn('scripts', t('Scripts'), 'script');
  railBtn('notes', t('Notes'), 'note');
  const sessionsSide = h(
    'div',
    { class: 'side-section', 'data-testid': 'section-sessions' },
    h('div', { class: 'side-head' }, h('span', {}, t('Sessions'))),
    h('div', { class: 'search' }, icon('plug', 14), quick),
    h('div', { class: 'search' }, icon('search', 14), search),
    list,
  );
  const sideBody = h('div', { class: 'side-body' });
  const openTabsEl = h('div', { class: 'open-tabs', 'data-testid': 'open-tabs' });
  sidebar.append(sideBody, openTabsEl);
  function showSection(name: SectionName): void {
    section = name;
    for (const [n, b] of railBtns) b.classList.toggle('active', n === name);
    clear(sideBody);
    sideBody.append(name === 'sessions' ? sessionsSide : sections[name].side);
    if (name !== 'sessions') void sections[name].refresh().then(renderHome);
    if (activeId !== 'home') activate('home');
    else renderHome();
    if (sidebar.classList.contains('hidden') && activeId === 'home') toggleSidebar();
  }
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
  onActivate = (id) => setSidebar(getSettings().tabs_position === 'left' ? false : id === 'home' ? pref.get('sidebar-hidden') === '1' : true);
  // ---------- status bar ----------
  const status = h(
    'footer',
    { class: 'statusbar' },
    h('span', { class: 'st' }, icon('unlock', 12), t('Vault unlocked')),
    statusCount,
    statusSessions,
    statusUpdate,
    h('div', { class: 'grow' }),
    h('span', {}, t('d256 Shell {0}', __APP_VERSION__)),
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
      if (await confirmDialog(t('Close d256 Shell?'), t('There are unsaved changes in open files. They will be lost.'), t('Close'), true)) await getCurrentWindow().destroy();
    });
  }

  showSection('sessions');
  await reload();
  if (getSettings().check_updates) {
    void autoCheck().then((r) => {
      if (!r) return;
      statusUpdate.style.display = '';
      statusUpdate.append(
        icon('download', 12),
        ' ' + t('Update {0} is available', r.latest) + ' ',
        h('button', { class: 'link', 'data-testid': 'update-release', onclick: () => void openRelease(r.url) }, t('Release page')),
        h('button', { class: 'icon-btn', title: t('Hide'), 'aria-label': t('Hide'), 'data-testid': 'update-hide', onclick: () => (dismissUpdate(r.latest), (statusUpdate.style.display = 'none')) }, icon('close', 10)),
      );
    });
  }
  // vertical tabs (Settings) and the profile that opens at start
  applyTabsPosition();
  onSettings(() => applyTabsPosition());
  const startup = getSettings().startup_profile;
  if (startup && startup !== 'home') {
    try {
      const p = (await api.shellProfiles()).profiles.find((x) => x.id === startup);
      if (p) openTerminal(p);
    } catch {
      /* the profile is gone: stay on the home tab */
    }
  }
}
