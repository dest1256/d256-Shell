// The SFTP manager tab: two panes (this computer on the left, the server on the right), each with a folder tree and a file
// list, a status log on top, the function keys (F2 ... F9) and the transfer queue at the bottom. The panes are the same
// explorer as everywhere else; this file adds the trees, the log, the queue, copy / move between the panes, comparison
// of the two folders and the quick filter.
import { api, ApiError, DirEntry, TransferView, transfers } from './api';
import { clear, confirmDialog, h, toast } from './dom';
import { EditorPane, pickEncoding } from './editor';
import { Explorer, formatSize, Mark } from './explorer';
import { icon } from './icons';
import { t, tm } from './i18n';
import { ScriptPane } from './localedit';
import { PaneSet } from './panes';
import { getSettings, onSettings, updateSettings } from './settings';
import { startTransfer, TransferPanel } from './transfers';
import { pref } from './util';

/** A path of the local explorer (`/C:/Users/me`) as the operating system writes it (`C:\Users\me`). */
export function osPath(ui: string): string {
  if (!/^\/[A-Za-z]:/.test(ui)) return ui;
  const p = ui.slice(1).replace(/\//g, '\\');
  return p.length === 2 ? p + '\\' : p;
}

const joinPath = (dir: string, name: string): string => (dir === '/' ? `/${name}` : `${dir}/${name}`);
const isDirEntry = (e: DirEntry): boolean => e.kind === 'dir' || (e.kind === 'symlink' && e.target_is_dir === true);

// ------------------------------------------------------------------ status log

class StatusLog {
  readonly el = h('div', { class: 'sm-log', 'data-testid': 'sm-log', role: 'log' });

  add(text: string, cls = ''): void {
    const now = new Date().toTimeString().slice(0, 8);
    this.el.append(h('div', {}, h('span', { class: 'sm-t' }, now), h('span', { class: 'sm-k' }, t('Status')), h('span', { class: cls }, text)));
    while (this.el.childElementCount > 500) this.el.firstElementChild?.remove();
    this.el.scrollTop = this.el.scrollHeight;
  }
}

// ------------------------------------------------------------------ folder tree

interface TNode {
  path: string;
  row: HTMLElement;
  box: HTMLElement;
  arrow: HTMLElement;
  /** The folders of this folder are being / were read (everyone waits for the same read). */
  reading: Promise<void> | null;
  open: boolean;
}

class FolderTree {
  readonly el: HTMLElement;
  private nodes = new Map<string, TNode>();
  private current = '';
  private seq = 0;

  constructor(
    private o: {
      testid: string;
      rootLabel: string;
      /** The names of the folders in a folder. */
      list: (path: string) => Promise<string[]>;
      onSelect: (path: string) => void;
    },
  ) {
    this.el = h('div', { class: 'sm-tree', 'data-testid': o.testid });
    const root = this.make('/', o.rootLabel, 0, this.el);
    root.open = true;
    void this.load(root);
  }

  private make(path: string, label: string, depth: number, parent: HTMLElement): TNode {
    const arrow = h('span', { class: 'arr' }, '▸');
    const row = h('div', { class: 'tn', 'data-path': path, 'data-testid': 'tree-node', style: `padding-left:${8 + depth * 14}px` }, arrow, h('span', { class: 'fi dir' }, icon('folder', 13)), h('span', { class: 'tl' }, label));
    const box = h('div', { class: 'tkids' });
    const n: TNode = { path, row, box, arrow, reading: null, open: false };
    arrow.addEventListener('click', (e) => {
      e.stopPropagation();
      void this.toggle(n);
    });
    row.addEventListener('click', () => {
      this.o.onSelect(path);
      if (!n.open) void this.toggle(n);
    });
    parent.append(row, box);
    this.nodes.set(path, n);
    box.style.display = 'none';
    return n;
  }

  private load(n: TNode): Promise<void> {
    n.reading ??= (async () => {
      let names: string[] = [];
      try {
        names = await this.o.list(n.path);
      } catch {
        names = [];
      }
      const depth = n.path === '/' ? 1 : n.path.split('/').filter(Boolean).length + 1;
      clear(n.box);
      for (const name of names) this.make(joinPath(n.path, name), name, depth, n.box);
      n.arrow.textContent = names.length ? (n.open ? '▾' : '▸') : '';
      this.paint();
    })();
    return n.reading;
  }

  private async toggle(n: TNode): Promise<void> {
    await this.load(n);
    n.open = !n.open;
    n.box.style.display = n.open ? '' : 'none';
    if (n.arrow.textContent) n.arrow.textContent = n.open ? '▾' : '▸';
  }

  private paint(): void {
    for (const [p, n] of this.nodes) n.row.classList.toggle('sel', p === this.current);
  }

  /** Open the folders down to `path` and mark it (the tree follows the file list). */
  async reveal(path: string): Promise<void> {
    const my = ++this.seq;
    this.current = path;
    let cur = '/';
    const root = this.nodes.get('/')!;
    await this.load(root);
    root.open = true;
    root.box.style.display = '';
    root.arrow.textContent = root.arrow.textContent ? '▾' : '';
    for (const part of path.split('/').filter(Boolean)) {
      if (my !== this.seq) return;
      cur = joinPath(cur, part);
      const n = this.nodes.get(cur);
      if (!n) break;
      await this.load(n);
      n.open = true;
      n.box.style.display = '';
      if (n.arrow.textContent) n.arrow.textContent = '▾';
    }
    if (my !== this.seq) return;
    this.paint();
    this.nodes.get(path)?.row.scrollIntoView({ block: 'nearest' });
  }

  /** Forget what was read (a refresh). */
  reset(): void {
    const root = this.nodes.get('/')!;
    for (const [p, n] of [...this.nodes]) if (p !== '/') (this.nodes.delete(p), n.row.remove(), n.box.remove());
    clear(root.box);
    root.reading = null;
    void this.load(root).then(() => { if (this.current) void this.reveal(this.current); });
  }
}

// ------------------------------------------------------------------ transfer queue

type QTab = 'queue' | 'failed' | 'done';

class QueuePanel {
  readonly el: HTMLElement;
  private tabs: Record<QTab, HTMLElement>;
  private body = h('div', { class: 'qbody', 'data-testid': 'sm-queue-body' });
  private tab: QTab = 'queue';
  private jobs: TransferView[] = [];
  private timer = 0;
  private seen = new Map<string, string>();

  constructor(
    private o: {
      sessionIds: () => string[];
      currentSession: () => string;
      onFinished: (tv: TransferView) => void;
      log: (msg: string, cls?: string) => void;
    },
  ) {
    const tabBtn = (k: QTab) => h('div', { class: 'qt', role: 'tab', 'data-testid': `sm-qtab-${k}`, onclick: () => ((this.tab = k), this.render()) });
    this.tabs = { queue: tabBtn('queue'), failed: tabBtn('failed'), done: tabBtn('done') };
    const buttons = h(
      'div',
      { class: 'qbtns' },
      h('button', { type: 'button', 'data-testid': 'sm-cancel-all', onclick: () => void this.cancelAll() }, t('Cancel all')),
      h('button', { type: 'button', 'data-testid': 'sm-clear', title: t('Clear finished'), onclick: () => void transfers.clear().then(() => this.kick()) }, t('Clear finished')),
    );
    this.el = h('div', { class: 'sm-queue', 'data-testid': 'sm-queue' }, h('div', { class: 'qtabs' }, this.tabs.queue, this.tabs.failed, this.tabs.done, h('span', { class: 'sp' }), buttons), this.body);
    this.render();
  }

  /** Poll while something runs (and once after it started or finished). */
  kick(): void {
    if (this.timer) return;
    const tick = async () => {
      this.timer = 0;
      let active = false;
      try {
        const all = await transfers.list();
        this.jobs = all.filter((j) => this.o.sessionIds().includes(j.session_id));
        active = this.jobs.some((j) => j.state === 'running' || j.state === 'queued');
        for (const j of this.jobs) {
          const was = this.seen.get(j.id);
          if (was === j.state) continue;
          this.seen.set(j.id, j.state);
          if (j.state === 'running' && was === undefined) this.o.log(t('Transfer started: {0}', j.label));
          if (j.state === 'done') this.o.log(t('Transfer finished: {0}', j.label), 'ok');
          if (j.state === 'error') this.o.log(t('Transfer failed: {0}: {1}', j.label, j.error ? tm(j.error) : ''), 'err');
          if (j.state === 'cancelled') this.o.log(t('Transfer cancelled: {0}', j.label));
          if (j.state === 'done' || j.state === 'error' || j.state === 'cancelled') this.o.onFinished(j);
        }
        this.render();
      } catch {
        /* the session is gone */
      }
      if (active) this.timer = window.setTimeout(tick, 400);
    };
    this.timer = window.setTimeout(tick, 50);
  }

  dispose(): void {
    window.clearTimeout(this.timer);
    this.timer = 1; // never again
  }

  private async cancelAll(): Promise<void> {
    for (const j of this.jobs) if (j.state === 'running' || j.state === 'queued') await transfers.cancel(j.id).catch(() => undefined);
    this.kick();
  }

  private group(k: QTab): TransferView[] {
    return this.jobs.filter((j) => (k === 'queue' ? j.state === 'running' || j.state === 'queued' : k === 'failed' ? j.state === 'error' || j.state === 'cancelled' : j.state === 'done'));
  }

  private render(): void {
    const names: Record<QTab, string> = { queue: t('Queued files'), failed: t('Failed transfers'), done: t('Successful transfers') };
    for (const k of ['queue', 'failed', 'done'] as QTab[]) {
      this.tabs[k].textContent = `${names[k]} (${this.group(k).length})`;
      this.tabs[k].classList.toggle('on', k === this.tab);
    }
    clear(this.body);
    const rows = this.group(this.tab);
    const table = h('table', { class: 'ft' }, h('thead', {}, h('tr', {}, h('th', {}, t('Name')), h('th', {}, t('Direction')), h('th', {}, t('Destination')), h('th', { class: 'r' }, t('Size')), h('th', {}, t('Status')))));
    const tb = h('tbody');
    for (const j of rows) tb.append(this.row(j));
    if (!rows.length) tb.append(h('tr', {}, h('td', { colspan: '5', class: 'muted' }, t('Nothing here.'))));
    table.append(tb);
    this.body.append(table);
  }

  private row(j: TransferView): HTMLElement {
    const pct = j.total > 0 ? Math.min(100, Math.round((j.done / j.total) * 100)) : j.state === 'done' ? 100 : 0;
    let status: HTMLElement | string;
    if (j.state === 'running') status = h('div', { class: 'prog' }, h('span', { style: `width:${pct}%` }), h('em', {}, `${pct}% · ${formatSize(j.speed)}/s`));
    else if (j.state === 'queued') status = t('Queued');
    else if (j.state === 'done') status = j.skipped.length ? t('Done, skipped: {0}', j.skipped.length) : t('Done');
    else status = h('span', { class: 'err' }, j.error ? tm(j.error) : j.state === 'cancelled' ? t('Cancelled') : t('Error'));
    const retry =
      j.state === 'error' || j.state === 'cancelled'
        ? h('button', { class: 'icon-btn', title: t('Retry'), 'data-testid': 'sm-retry', onclick: () => void transfers.retry(j.id, { session_id: this.o.currentSession() }).then(() => this.kick()).catch((e) => toast(e instanceof Error ? e.message : String(e))) }, icon('refresh', 13))
        : null;
    const cancel = j.state === 'running' || j.state === 'queued' ? h('button', { class: 'icon-btn', title: t('Cancel'), 'data-testid': 'sm-cancel', onclick: () => void transfers.cancel(j.id).then(() => this.kick()) }, icon('close', 13)) : null;
    return h(
      'tr',
      { 'data-testid': 'sm-job', 'data-state': j.state, 'data-kind': j.kind },
      h('td', { title: j.label }, j.label),
      h('td', {}, j.kind === 'upload' ? '→ ' + t('to the server') : '← ' + t('to this computer')),
      h('td', { class: 'm', title: j.dest }, j.dest),
      h('td', { class: 'r m' }, j.total ? formatSize(j.total) : ''),
      h('td', {}, status, retry, cancel),
    );
  }
}

// ------------------------------------------------------------------ the manager

type Side = 'L' | 'R';

interface PaneUi {
  side: Side;
  el: HTMLElement;
  explorer: Explorer;
  tree: FolderTree;
  filter: HTMLInputElement;
}

export interface SftpManagerOpts {
  sessionId: string;
  /** Log in again; the new session id, or null. */
  reconnect: () => Promise<string | null>;
  /** F9: the terminal of this computer or of the server. */
  openTerminal: (side: 'local' | 'remote') => void;
  /** Alt+X. */
  closeTab: () => void;
  initialDir?: string | null;
}

const FKEYS: { key: string; label: () => string }[] = [
  { key: 'F3', label: () => t('View') },
  { key: 'F4', label: () => t('Edit') },
  { key: 'F5', label: () => t('Copy') },
  { key: 'F6', label: () => t('Move') },
  { key: 'F7', label: () => t('New folder') },
  { key: 'F8', label: () => t('Delete') },
  { key: 'F2', label: () => t('Rename') },
  { key: 'F9', label: () => t('Terminal') },
  { key: 'Alt+X', label: () => t('Close tab') },
];

export class SftpManager {
  /** The tab's content: the pane strip (the files, then one pane per open file). */
  readonly el: HTMLElement;
  private panes = new PaneSet();
  private root: HTMLElement;
  private log = new StatusLog();
  private queue: QueuePanel;
  private sessions = new Set<string>();
  private session: string;
  private ui!: Record<Side, PaneUi>;
  private active: Side = 'L';
  private compareOn = false;
  private btns: Record<'compare' | 'filter' | 'log' | 'trees', HTMLElement>;
  private keysBar = h('div', { class: 'fkeys', 'data-testid': 'sm-fkeys' });
  private offSettings: () => void;

  constructor(private o: SftpManagerOpts) {
    this.session = o.sessionId;
    this.sessions.add(o.sessionId);
    this.queue = new QueuePanel({
      sessionIds: () => [...this.sessions],
      currentSession: () => this.session,
      log: (m, c) => this.log.add(m, c),
      onFinished: (tv) => {
        // the pane that received files shows them
        void (tv.kind === 'upload' ? this.ui.R.explorer.reload() : this.ui.L.explorer.reload());
      },
    });
    const tbtn = (name: string, label: string, testid: string, fn: () => void) => h('button', { class: 'sm-btn', type: 'button', 'data-testid': testid, title: label, onclick: fn }, icon(name, 14), ' ' + label);
    this.btns = {
      compare: tbtn('activity', t('Compare'), 'sm-compare', () => this.setCompare(!this.compareOn)),
      filter: tbtn('search', t('Filter'), 'sm-filter', () => this.setFilterOn(!this.btns.filter.classList.contains('on'))),
      log: tbtn('file', t('Log'), 'sm-toggle-log', () => void updateSettings({ sftp_log: !getSettings().sftp_log })),
      trees: tbtn('folder', t('Trees'), 'sm-toggle-trees', () => void updateSettings({ sftp_trees: !getSettings().sftp_trees })),
    };
    const toolbar = h(
      'div',
      { class: 'sm-tb' },
      tbtn('refresh', t('Refresh'), 'sm-refresh', () => this.refresh()),
      tbtn('terminal', t('Terminal'), 'sm-terminal', () => this.command('F9')),
      h('span', { class: 'sep' }),
      this.btns.compare,
      this.btns.filter,
      h('span', { class: 'sep' }),
      this.btns.log,
      this.btns.trees,
    );
    this.ui = { L: this.makePane('L'), R: this.makePane('R') };
    for (const f of FKEYS) this.keysBar.append(h('button', { type: 'button', 'data-testid': `fk-${f.key}`, onclick: () => this.command(f.key) }, h('b', {}, f.key), f.label()));
    const split = h('div', { class: 'sm-split' });
    this.root = h('div', { class: 'sm', 'data-testid': 'sftp-manager' }, toolbar, this.log.el, h('div', { class: 'sm-panes' }, this.ui.L.el, split, this.ui.R.el), this.keysBar, this.queue.el);
    this.root.addEventListener('keydown', (e) => this.onKey(e), true);
    this.panes.add({ id: 'files', title: t('Files'), view: h('div', { class: 'sm-wrap' }, this.root), closable: false, onShow: () => this.ui[this.active].explorer.focusList() });
    this.el = this.panes.el;
    this.applySettings();
    this.offSettings = onSettings(() => this.applySettings());
    this.markActive();
    this.log.add(t('SFTP session open'), 'ok');
    // the left pane starts where it was last time (the home folder at the first start)
    void this.ui.L.explorer.load(pref.get('sftp-local') || undefined);
    void this.ui.R.explorer.load(o.initialDir || undefined);
    void this.queue.kick();
  }

  // ---------------------------------------------------------- panes

  private makePane(side: Side): PaneUi {
    const local = side === 'L';
    const sessionOf = () => this.session;
    const explorer: Explorer = new Explorer(
      local ? 'local' : this.session,
      {
        compact: true,
        onOpenFile: (path) => void this.openFile(side, path),
        onNewFile: (path) => void this.newFile(side, path),
        onReconnect: () => this.reconnect(),
        onTransfer: () => this.queue.kick(),
        onCopy: () => void this.transfer(false),
        onMove: () => void this.transfer(true),
        onLog: (m) => this.log.add(m),
        onFocus: () => ((this.active = side), this.markActive()),
        onListed: () => this.afterListing(),
        onPathChange: (p) => {
          if (local) pref.set('sftp-local', p);
          void ui.tree.reveal(p);
        },
      },
      local,
    );
    const tree = new FolderTree({
      testid: local ? 'tree-local' : 'tree-remote',
      rootLabel: local ? t('This computer') : '/',
      list: async (path) => {
        const l = local ? await api.lfsList(path) : await api.listDir(sessionOf(), path);
        return l.entries.filter(isDirEntry).map((e) => e.name).sort((a, b) => a.localeCompare(b, undefined, { numeric: true, sensitivity: 'base' }));
      },
      onSelect: (path) => void explorer.load(path),
    });
    const filter = h('input', { class: 'sm-filter', type: 'text', placeholder: t('Filter by name…'), 'data-testid': local ? 'filter-local' : 'filter-remote', 'aria-label': t('Filter by name…') }) as HTMLInputElement;
    filter.addEventListener('input', () => {
      explorer.setFilter(filter.value);
    });
    filter.addEventListener('keydown', (e) => {
      if (e.key === 'Escape') {
        filter.value = '';
        explorer.setFilter('');
        explorer.focusList();
      }
    });
    const el = h(
      'section',
      { class: 'sm-pane', 'data-testid': local ? 'pane-local' : 'pane-remote' },
      h('div', { class: 'sm-phead' }, h('b', {}, local ? t('Local site:') : t('Remote site:')), filter),
      tree.el,
      explorer.el,
    );
    el.addEventListener('pointerdown', () => ((this.active = side), this.markActive()), true);
    const ui: PaneUi = { side, el, explorer, tree, filter };
    return ui;
  }

  private markActive(): void {
    for (const s of ['L', 'R'] as Side[]) this.ui?.[s]?.el.classList.toggle('active', s === this.active);
  }

  private switchPane(): void {
    this.active = this.active === 'L' ? 'R' : 'L';
    this.markActive();
    this.ui[this.active].explorer.focusList();
  }

  private applySettings(): void {
    const st = getSettings();
    this.log.el.classList.toggle('off', !st.sftp_log);
    this.btns.log.classList.toggle('on', st.sftp_log);
    for (const s of ['L', 'R'] as Side[]) this.ui[s].tree.el.classList.toggle('off', !st.sftp_trees);
    this.btns.trees.classList.toggle('on', st.sftp_trees);
    this.keysBar.classList.toggle('off', !st.sftp_keys);
  }

  private refresh(): void {
    this.ui.L.tree.reset();
    this.ui.R.tree.reset();
    void this.ui.L.explorer.reload();
    void this.ui.R.explorer.reload();
  }

  private async reconnect(): Promise<boolean> {
    const id = await this.o.reconnect();
    if (!id) return false;
    this.session = id;
    this.sessions.add(id);
    this.ui.R.explorer.setSession(id);
    this.log.add(t('Reconnected'), 'ok');
    return true;
  }

  // ---------------------------------------------------------- comparison and filter

  private setCompare(on: boolean): void {
    this.compareOn = on;
    this.btns.compare.classList.toggle('on', on);
    this.root.classList.toggle('comparing', on);
    this.afterListing(true);
  }

  private setFilterOn(on: boolean): void {
    this.btns.filter.classList.toggle('on', on);
    for (const s of ['L', 'R'] as Side[]) {
      const f = this.ui[s].filter;
      f.classList.toggle('on', on);
      if (!on && f.value) {
        f.value = '';
        this.ui[s].explorer.setFilter('');
      }
    }
    if (on) this.ui[this.active].filter.focus();
  }

  /** A folder was read: when the comparison is on, colour both lists again. */
  private afterListing(announce = false): void {
    const L = this.ui?.L?.explorer;
    const R = this.ui?.R?.explorer;
    if (!L || !R) return;
    if (!this.compareOn) {
      L.setMarks(new Map());
      R.setMarks(new Map());
      return;
    }
    const a = new Map(L.entryList().map((e) => [e.name, e]));
    const b = new Map(R.entryList().map((e) => [e.name, e]));
    const ma = new Map<string, Mark>();
    const mb = new Map<string, Mark>();
    let different = 0;
    let single = 0;
    for (const name of new Set([...a.keys(), ...b.keys()])) {
      const x = a.get(name);
      const y = b.get(name);
      if (!x || !y) {
        (x ? ma : mb).set(name, 'only');
        single++;
        continue;
      }
      if (isDirEntry(x) && isDirEntry(y)) continue;
      if (isDirEntry(x) !== isDirEntry(y) || x.size !== y.size) {
        ma.set(name, 'diff');
        mb.set(name, 'diff');
        different++;
      } else if (x.mtime !== null && y.mtime !== null && Math.abs(x.mtime - y.mtime) > 2) {
        (x.mtime > y.mtime ? ma : mb).set(name, 'newer');
        different++;
      }
    }
    L.setMarks(ma);
    R.setMarks(mb);
    if (announce) this.log.add(t('The folders are compared: {0} differ, {1} exist on one side only', different, single));
  }

  // ---------------------------------------------------------- the function keys

  private onKey(e: KeyboardEvent): void {
    if ((e.target as HTMLElement).closest('input, textarea, select')) return;
    if (e.altKey && e.key.toLowerCase() === 'x') {
      e.preventDefault();
      e.stopPropagation();
      return this.o.closeTab();
    }
    if (e.ctrlKey || e.altKey || e.metaKey) return;
    if (e.key === 'Tab') {
      e.preventDefault();
      e.stopPropagation();
      return this.switchPane();
    }
    if (/^F[2-9]$/.test(e.key)) {
      e.preventDefault();
      e.stopPropagation();
      this.command(e.key);
    }
  }

  private command(key: string): void {
    const pane = this.ui[this.active];
    switch (key) {
      case 'F2':
        void pane.explorer.renameSelected();
        break;
      case 'F3':
      case 'F4': {
        const e = pane.explorer.cursorEntry();
        if (!e) return void toast(t('Select a file first.'));
        pane.explorer.openCursor();
        break;
      }
      case 'F5':
        void this.transfer(false);
        break;
      case 'F6':
        void this.transfer(true);
        break;
      case 'F7':
        void pane.explorer.newFolder();
        break;
      case 'F8':
        void pane.explorer.deleteSelected();
        break;
      case 'F9':
        this.o.openTerminal(this.active === 'L' ? 'local' : 'remote');
        break;
      case 'Alt+X':
        this.o.closeTab();
        break;
    }
  }

  /** F5 / F6: the selection of the active pane goes to the folder of the other pane (a move deletes the originals after it). */
  private async transfer(move: boolean): Promise<void> {
    const from = this.ui[this.active];
    const to = this.ui[this.active === 'L' ? 'R' : 'L'];
    const items = from.explorer.selectedItems();
    if (!items.length) return void toast(t('Select files or folders first.'));
    const upload = this.active === 'L';
    const names = items.length === 1 ? `“${items[0].entry.name}”` : t('{0} objects', items.length);
    if (move) {
      const ok = await confirmDialog(t('Move?'), upload ? t('{0} will be copied to the server and then moved to the trash here.', names) : t('{0} will be copied to this computer and then deleted on the server.', names), t('Move'), true);
      if (!ok) return;
    }
    const dest = to.explorer.currentPath;
    const id = upload ? await startTransfer(this.session, 'upload', items.map((i) => osPath(i.path)), dest) : await startTransfer(this.session, 'download', items.map((i) => i.path), osPath(dest));
    if (!id) return;
    this.log.add(upload ? t('Queued for upload: {0}', names) : t('Queued for download: {0}', names));
    this.queue.kick();
    if (!move) return;
    // delete the originals only when everything arrived
    try {
      const job = await TransferPanel.wait(id);
      if (job.state !== 'done' || job.skipped.length) return void this.log.add(t('Not everything was copied: the originals stay'), 'err');
      const paths = items.map((i) => i.path);
      const r = upload ? await api.lfs<{ errors: unknown[] }>('trash', { paths }) : await api.fs<{ errors: unknown[] }>(this.session, 'delete', { paths });
      this.log.add(r.errors.length ? t('Some originals could not be deleted') : t('The originals are deleted'), r.errors.length ? 'err' : 'ok');
      await from.explorer.reload();
    } catch (e) {
      toast(e instanceof ApiError ? e.message : String(e instanceof Error ? e.message : e));
    }
  }

  // ---------------------------------------------------------- editors

  private async openFile(side: Side, path: string): Promise<void> {
    const pid = `f:${side}:${path}`;
    if (this.panes.has(pid)) return this.panes.activate(pid);
    const name = path.slice(path.lastIndexOf('/') + 1);
    if (side === 'L') {
      const item = { id: 'file:' + path, name, ext: name.includes('.') ? name.slice(name.lastIndexOf('.') + 1).toLowerCase() : '', kind: 'file' as const, path, exists: true };
      const ed = new ScriptPane(item, { onDirty: (d) => this.panes.setDirty(pid, d), onClose: () => void this.panes.close(pid) }, () => undefined);
      if (!(await ed.open())) return;
      this.panes.add({
        id: pid,
        title: name,
        view: ed.el,
        closable: true,
        onShow: () => ed.focus(),
        beforeClose: async () => !ed.dirty || (await confirmDialog(t('Close without saving?'), t('“{0}” has unsaved changes. They will be lost.', path), t('Close'), true)),
      });
      ed.focus();
      return;
    }
    const ed = new EditorPane(path, {
      session: () => this.session,
      reconnect: () => this.reconnect(),
      onDirty: (d) => this.panes.setDirty(pid, d),
      onClose: () => void this.panes.close(pid),
    });
    let failure: unknown = null;
    try {
      await ed.load(false);
    } catch (e0) {
      failure = e0;
      if (e0 instanceof ApiError && e0.code === 'encoding') {
        const enc = await pickEncoding(e0.message);
        if (enc) {
          try {
            await ed.load(false, enc);
            failure = null;
          } catch (e2) {
            failure = e2;
          }
        }
      }
    }
    if (failure !== null) {
      ed.dispose();
      const e = failure;
      if (e instanceof ApiError && (e.code === 'binary' || e.code === 'too_large')) {
        if (await confirmDialog(t('The file cannot be opened in the editor'), t('{0}\n\nDownload “{1}”?', e.message, name), t('Download…'))) await this.ui.R.explorer.downloadPaths([path]);
      } else toast(e instanceof ApiError ? e.message : String(e), e instanceof ApiError ? e.code : undefined);
      return;
    }
    ed.el.addEventListener('saved', () => void this.ui.R.explorer.reload());
    this.panes.add({
      id: pid,
      title: name,
      view: ed.el,
      closable: true,
      onShow: () => ed.focus(),
      dispose: () => ed.dispose(),
      beforeClose: async () => !ed.dirty || (await confirmDialog(t('Close without saving?'), t('“{0}” has unsaved changes. They will be lost.', path), t('Close'), true)),
    });
    ed.focus();
  }

  private async newFile(side: Side, path: string): Promise<void> {
    if (side === 'R') {
      // created by the first save in the editor
      const pid = `f:R:${path}`;
      if (this.panes.has(pid)) return this.panes.activate(pid);
      const ed = new EditorPane(path, { session: () => this.session, reconnect: () => this.reconnect(), onDirty: (d) => this.panes.setDirty(pid, d), onClose: () => void this.panes.close(pid) });
      try {
        await ed.load(true);
      } catch (e) {
        ed.dispose();
        return void toast(e instanceof ApiError ? e.message : String(e));
      }
      ed.el.addEventListener('saved', () => void this.ui.R.explorer.reload());
      this.panes.add({ id: pid, title: path.slice(path.lastIndexOf('/') + 1), view: ed.el, closable: true, onShow: () => ed.focus(), dispose: () => ed.dispose() });
      return;
    }
    const i = path.lastIndexOf('/');
    try {
      await api.lfs('touch', { parent: path.slice(0, i) || '/', name: path.slice(i + 1) });
    } catch (e) {
      return void toast(e instanceof ApiError ? e.message : String(e), e instanceof ApiError ? e.code : undefined);
    }
    await this.ui.L.explorer.reload();
    await this.openFile('L', path);
  }

  // ---------------------------------------------------------- the tab

  dirty(): boolean {
    return this.panes.anyDirty();
  }

  show(): void {
    this.panes.activate(this.panes.active || 'files');
  }

  dispose(): void {
    this.offSettings();
    this.queue.dispose();
    this.panes.disposeAll();
  }
}
