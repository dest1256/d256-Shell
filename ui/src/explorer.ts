// Remote file explorer (SFTP): browse, create, rename, copy/move, delete, chmod. The list is virtualized: only visible rows exist in the DOM,
// so folders with 10 000+ entries stay responsive.
import { api, ApiError, DirEntry, transfers } from './api';
import { chmodDialog, propertiesDialog } from './explorer-extra';
import { native } from './native';
import { startTransfer, TransferPanel } from './transfers';
import { clear, confirmDialog, h, modal, promptDialog, toast } from './dom';
import { icon } from './icons';
import { pref } from './util';

const ROW = 26;

interface BatchResult {
  done: string[];
  errors: { path: string; message: string }[];
}
const collator = new Intl.Collator('en', { numeric: true, sensitivity: 'base' });

type SortKey = 'name' | 'size' | 'mtime' | 'mode';

export function formatSize(n: number): string {
  if (n < 1024) return `${n} B`;
  const u = ['KB', 'MB', 'GB', 'TB'];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < u.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v >= 100 ? v.toFixed(0) : v.toFixed(1)} ${u[i]}`;
}

export function modeString(mode: number | null, kind: DirEntry['kind']): string {
  if (mode === null) return '';
  const t = kind === 'dir' ? 'd' : kind === 'symlink' ? 'l' : kind === 'file' ? '-' : '?';
  const bits = 'rwxrwxrwx';
  let s = '';
  for (let i = 0; i < 9; i++) s += mode & (1 << (8 - i)) ? bits[i] : '-';
  const patch = (idx: number, on: boolean, lo: string, up: string) => {
    if (on) s = s.slice(0, idx) + (s[idx] === 'x' ? lo : up) + s.slice(idx + 1);
  };
  patch(2, !!(mode & 0o4000), 's', 'S');
  patch(5, !!(mode & 0o2000), 's', 'S');
  patch(8, !!(mode & 0o1000), 't', 'T');
  return t + s;
}

function formatTime(sec: number | null): string {
  if (sec === null) return '';
  return new Date(sec * 1000).toLocaleString('en-US', { year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' });
}

const isDirLike = (e: DirEntry) => e.kind === 'dir' || (e.kind === 'symlink' && e.target_is_dir === true);

/** The "go up" row shown above all folders (like MobaXterm); it is not a real entry and takes part in no operation. */
const UP = '..';
const PARENT_ENTRY: DirEntry = { name: UP, kind: 'dir', target_is_dir: null, size: 0, mtime: null, mode: null, uid: null, gid: null, owner: null, group: null, name_lossy: false };

export interface ExplorerHooks {
  /** Called for a double click / Enter on a regular file (editor arrives in a later stage). */
  onOpenFile?: (path: string, entry: DirEntry) => void;
  /** Create a new, not yet existing file in the editor (it is created by the first save). */
  onNewFile?: (path: string) => void;
  /** Re-open the SFTP session; true when it works again. Called by the banner button and automatically on the next operation. */
  onReconnect?: () => Promise<boolean> | void;
  onPathChange?: (path: string) => void;
}

export class Explorer {
  /** Copy/cut buffer shared by all explorers (paste works inside one server session). */
  static clipboard: { session: string; cut: boolean; dir: string; names: string[] } | null = null;

  readonly el: HTMLElement;
  private path = '';
  private entries: DirEntry[] = [];
  private view: DirEntry[] = [];
  private sortKey: SortKey = 'name';
  private asc = true;
  private showHidden = pref.get('show-hidden') !== '0'; // dotfiles are shown by default
  private selected = new Set<string>();
  private cursor = -1;
  private seq = 0;

  private pathInput = h('input', { class: 'path-input', 'data-testid': 'path-input', spellcheck: 'false', 'aria-label': 'Path' }) as HTMLInputElement;
  private body = h('div', { class: 'vbody', tabindex: '0', 'data-testid': 'explorer-list' });
  private size = h('div', { class: 'vsize' });
  private status = h('div', { class: 'ex-status', 'data-testid': 'explorer-status' });
  private banner = h('div', { class: 'ex-banner', 'data-testid': 'explorer-banner' });
  private headers: Record<SortKey, HTMLElement>;
  private hiddenBtn: HTMLElement;
  private raf = 0;
  private xfer: TransferPanel;
  private crumbs = h('div', { class: 'ex-crumbs', 'data-testid': 'crumbs' });
  private sessionIds = new Set<string>();
  private suppressClick = false;
  private dragOutAbort: { aborted: boolean } | null = null;
  private static byEl = new WeakMap<Element, Explorer>();
  private static dropReady = false;
  private static noAskMove = new Set<string>();

  /** After a reconnect the explorer keeps its state but talks to the new session. */
  setSession(id: string): void {
    this.sessionId = id;
    this.sessionIds.add(id);
  }

  /** The server has no SFTP (disabled or forbidden): explain instead of showing an empty list. */
  showUnavailable(message: string): void {
    this.el.classList.add('unavailable');
    clear(this.banner);
    this.banner.classList.add('show', 'info');
    this.banner.append(h('span', { 'data-testid': 'sftp-unavailable' }, message));
    this.entries = [];
    this.rebuild();
    this.status.textContent = 'File panel unavailable';
  }

  constructor(
    private sessionId: string,
    private hooks: ExplorerHooks = {},
  ) {
    const btn = (name: string, title: string, testid: string, fn: () => void) =>
      h('button', { class: 'icon-btn', title, 'aria-label': title, 'data-testid': testid, onclick: fn }, icon(name, 15));
    this.hiddenBtn = btn('eye', 'Show hidden files', 'hidden-toggle', () => {
      this.showHidden = !this.showHidden;
      pref.set('show-hidden', this.showHidden ? '1' : '0');
      this.hiddenBtn.classList.toggle('on', this.showHidden);
      this.rebuild();
    });
    const col = (key: SortKey, label: string) => {
      const el = h('button', { class: 'th', 'data-testid': `sort-${key}`, onclick: () => this.setSort(key) }, label);
      return el;
    };
    this.headers = { name: col('name', 'Name'), size: col('size', 'Size'), mtime: col('mtime', 'Modified'), mode: col('mode', 'Permissions') };

    this.pathInput.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') void this.load(this.pathInput.value.trim());
      if (e.key === 'Escape') this.pathInput.value = this.path;
    });
    this.body.append(this.size);
    this.body.addEventListener('scroll', () => this.scheduleRender());
    this.body.addEventListener('keydown', (e) => this.onKey(e));
    this.body.addEventListener('contextmenu', (e) => {
      e.preventDefault();
      this.selected.clear();
      this.render();
      this.updateStatus();
      this.showMenu(e.clientX, e.clientY, false);
    });
    new ResizeObserver(() => this.scheduleRender()).observe(this.body);
    this.sessionIds.add(sessionId);
    this.xfer = new TransferPanel({ sessionIds: () => [...this.sessionIds], onFinished: (t) => this.onTransferFinished(t), currentSession: () => this.sessionId });
    this.attachDrag();

    this.el = h(
      'div',
      { class: 'explorer', 'data-testid': 'explorer' },
      h(
        'div',
        { class: 'ex-bar' },
        btn('up', 'Up (Backspace)', 'up', () => void this.load(this.path + '/..')),
        btn('home', 'Home folder', 'ex-home', () => void this.load()),
        btn('refresh', 'Refresh (F5)', 'refresh', () => void this.load(this.path)),
        this.pathInput,
        this.hiddenBtn,
      ),
      h(
        'div',
        { class: 'ex-bar ex-actions' },
        btn('folder-plus', 'New folder (Ctrl+Shift+N)', 'new-folder', () => void this.newFolder()),
        btn('file-plus', 'New file (Ctrl+N)', 'new-file', () => void this.newFile()),
        btn('upload', 'Upload files to the server…', 'upload', () => void this.uploadHere('files')),
        btn('download', 'Download selected…', 'download', () => void this.downloadSelected()),
      ),
      this.crumbs,
      this.banner,
      h('div', { class: 'ex-head' }, this.headers.name, this.headers.size, this.headers.mtime, this.headers.mode),
      this.body,
      this.xfer.el,
      this.status,
    );
    this.updateHeaders();
    this.hiddenBtn.classList.toggle('on', this.showHidden);
    Explorer.byEl.set(this.el, this);
    Explorer.initDrop();
  }

  get currentPath(): string {
    return this.path;
  }

  private setSort(key: SortKey): void {
    if (this.sortKey === key) this.asc = !this.asc;
    else {
      this.sortKey = key;
      this.asc = true;
    }
    this.updateHeaders();
    this.rebuild();
  }

  private updateHeaders(): void {
    const label: Record<SortKey, string> = { name: 'Name', size: 'Size', mtime: 'Modified', mode: 'Permissions' };
    for (const k of Object.keys(this.headers) as SortKey[]) {
      this.headers[k].textContent = label[k] + (k === this.sortKey ? (this.asc ? ' ▲' : ' ▼') : '');
    }
  }

  async load(path?: string): Promise<void> {
    const my = ++this.seq;
    this.el.classList.add('loading');
    try {
      const l = await this.retrying(() => api.listDir(this.sessionId, path));
      if (my !== this.seq) return;
      this.path = l.path;
      this.entries = l.entries;
      this.pathInput.value = l.path;
      this.renderCrumbs();
      this.selected.clear();
      this.cursor = -1;
      this.banner.textContent = '';
      this.banner.classList.remove('show');
      this.body.scrollTop = 0;
      this.rebuild();
      this.hooks.onPathChange?.(l.path);
    } catch (e) {
      if (my !== this.seq) return;
      this.pathInput.value = this.path;
      if (e instanceof ApiError && e.code === 'sftp_unavailable') {
        this.showUnavailable(e.message);
      } else if (e instanceof ApiError && (e.code === 'connection_lost' || e.code === 'no_session')) {
        this.showLost();
      } else {
        toast(e instanceof ApiError ? e.message : String(e), e instanceof ApiError ? `${e.code}\n${JSON.stringify(e.details ?? {})}` : undefined);
      }
    } finally {
      if (my === this.seq) this.el.classList.remove('loading');
    }
  }

  private showLost(): void {
    clear(this.banner);
    this.banner.classList.add('show');
    this.banner.append(h('span', {}, 'Connection to the server lost.'), h('button', { class: 'link', onclick: () => this.hooks.onReconnect?.(), 'data-testid': 'reconnect' }, 'Reconnect'));
  }

  private rebuild(): void {
    const dirs = (e: DirEntry) => (isDirLike(e) ? 0 : 1);
    const dir = this.asc ? 1 : -1;
    const v = this.showHidden ? [...this.entries] : this.entries.filter((e) => !e.name.startsWith('.'));
    v.sort((a, b) => {
      const g = dirs(a) - dirs(b);
      if (g) return g; // folders always first
      let c = 0;
      switch (this.sortKey) {
        case 'size':
          c = a.size - b.size;
          break;
        case 'mtime':
          c = (a.mtime ?? 0) - (b.mtime ?? 0);
          break;
        case 'mode':
          c = (a.mode ?? 0) - (b.mode ?? 0);
          break;
      }
      return (c || collator.compare(a.name, b.name)) * dir;
    });
    if (this.path && this.path !== '/') v.unshift(PARENT_ENTRY);
    this.view = v;
    this.size.style.height = `${this.view.length * ROW}px`;
    this.render();
    this.updateStatus();
  }

  private updateStatus(): void {
    const real = this.view.filter((e) => e.name !== UP).length;
    const hidden = this.entries.length - real;
    let t = `Objects: ${real}`;
    if (!this.showHidden && hidden > 0) t += ` (hidden: ${hidden})`;
    const sel = [...this.selected].filter((n) => n !== UP).length;
    if (sel) t += ` · selected: ${sel}`;
    this.status.textContent = t;
  }

  private scheduleRender(): void {
    if (this.raf) return;
    this.raf = requestAnimationFrame(() => {
      this.raf = 0;
      this.render();
    });
  }

  private render(): void {
    const top = this.body.scrollTop;
    const h0 = this.body.clientHeight || 400;
    const first = Math.max(0, Math.floor(top / ROW) - 6);
    const last = Math.min(this.view.length, Math.ceil((top + h0) / ROW) + 6);
    // keep the sizer, replace rows
    while (this.body.childNodes.length > 1) this.body.removeChild(this.body.lastChild!);
    for (let i = first; i < last; i++) this.body.append(this.row(i));
  }

  private row(i: number): HTMLElement {
    const e = this.view[i];
    const dirLike = isDirLike(e);
    const ico = e.name === UP ? 'up' : dirLike ? 'folder' : e.kind === 'symlink' ? 'link' : 'file';
    const el = h(
      'div',
      {
        class: 'vrow' + (this.selected.has(e.name) ? ' selected' : '') + (i === this.cursor ? ' cursor' : '') + (e.name_lossy ? ' lossy' : ''),
        'data-testid': 'explorer-row',
        'data-name': e.name,
        'data-kind': e.kind,
        'data-droppable': dirLike && !e.name_lossy ? '1' : '0',
        title: e.name === UP ? 'One level up (Backspace)' : e.name_lossy ? 'The name contains bytes that are not UTF-8. Operations on such a file are unavailable.' : e.name,
        onclick: (ev) => this.onClickRow(i, ev as MouseEvent),
        ondblclick: () => this.open(e),
        oncontextmenu: (ev) => {
          ev.preventDefault();
          ev.stopPropagation();
          if (e.name === UP) {
            this.selected.clear();
            this.render();
            this.updateStatus();
            return this.showMenu((ev as MouseEvent).clientX, (ev as MouseEvent).clientY, false);
          }
          if (!this.selected.has(e.name)) {
            this.selected.clear();
            this.selected.add(e.name);
            this.cursor = i;
            this.render();
            this.updateStatus();
          }
          this.showMenu((ev as MouseEvent).clientX, (ev as MouseEvent).clientY, true);
        },
      },
      h('div', { class: 'c-name' }, h('span', { class: 'fi ' + (dirLike ? 'dir' : '') }, icon(ico, 15)), h('span', { class: 'nm' }, e.name), e.kind === 'symlink' ? h('span', { class: 'tag' }, '→') : null, e.name_lossy ? h('span', { class: 'tag warn', title: 'non-UTF-8 name' }, '⚠') : null),
      h('div', { class: 'c-size' }, e.kind === 'file' ? formatSize(e.size) : ''),
      h('div', { class: 'c-time' }, formatTime(e.mtime)),
      h('div', { class: 'c-mode' }, modeString(e.mode, e.kind)),
    );
    el.style.top = `${i * ROW}px`;
    return el;
  }

  private onClickRow(i: number, ev: MouseEvent): void {
    if (this.suppressClick) return;
    const name = this.view[i].name;
    if (ev.shiftKey && this.cursor >= 0) {
      const [a, b] = [Math.min(this.cursor, i), Math.max(this.cursor, i)];
      if (!ev.ctrlKey) this.selected.clear();
      for (let k = a; k <= b; k++) this.selected.add(this.view[k].name);
    } else if (ev.ctrlKey) {
      if (!this.selected.delete(name)) this.selected.add(name);
      this.cursor = i;
    } else {
      this.selected.clear();
      this.selected.add(name);
      this.cursor = i;
    }
    this.body.focus();
    this.render();
    this.updateStatus();
  }

  private join(name: string): string {
    return this.path === '/' ? `/${name}` : `${this.path}/${name}`;
  }

  private open(e: DirEntry): void {
    if (e.name === UP) return void this.load(this.path + '/..');
    if (e.name_lossy) {
      toast('The file name contains bytes that are not UTF-8: operations on it are unavailable so that another file is not affected.');
      return;
    }
    if (isDirLike(e)) void this.load(this.join(e.name));
    else if (e.kind === 'symlink' && e.target_is_dir === null) toast('The link points to nothing (a broken link).');
    else this.hooks.onOpenFile?.(this.join(e.name), e);
  }

  private move(to: number, extend: boolean): void {
    if (!this.view.length) return;
    const n = Math.max(0, Math.min(this.view.length - 1, to));
    if (!extend) this.selected.clear();
    this.selected.add(this.view[n].name);
    this.cursor = n;
    // keep the cursor row visible
    const y = n * ROW;
    if (y < this.body.scrollTop) this.body.scrollTop = y;
    else if (y + ROW > this.body.scrollTop + this.body.clientHeight) this.body.scrollTop = y + ROW - this.body.clientHeight;
    this.render();
    this.updateStatus();
  }

  private onKey(e: KeyboardEvent): void {
    switch (e.key) {
      case 'ArrowDown':
        this.move(this.cursor + 1, e.shiftKey);
        break;
      case 'ArrowUp':
        this.move(this.cursor < 0 ? 0 : this.cursor - 1, e.shiftKey);
        break;
      case 'Home':
        this.move(0, e.shiftKey);
        break;
      case 'End':
        this.move(this.view.length - 1, e.shiftKey);
        break;
      case 'PageDown':
        this.move(this.cursor + Math.floor(this.body.clientHeight / ROW), e.shiftKey);
        break;
      case 'PageUp':
        this.move(this.cursor - Math.floor(this.body.clientHeight / ROW), e.shiftKey);
        break;
      case 'Enter':
        if (this.cursor >= 0) this.open(this.view[this.cursor]);
        break;
      case 'Backspace':
        void this.load(this.path + '/..');
        break;
      case 'F5':
        void this.load(this.path);
        break;
      case 'F2':
        void this.renameSelected();
        break;
      case 'Delete':
        void this.deleteSelected();
        break;
      default: {
        if (!e.ctrlKey || e.altKey) return;
        const k = e.key.toLowerCase();
        if (k === 'a') {
          for (const v of this.view) if (v.name !== UP) this.selected.add(v.name);
          this.render();
          this.updateStatus();
        } else if (k === 'c') this.copySelected(false);
        else if (k === 'x') this.copySelected(true);
        else if (k === 'v') void this.paste();
        else if (k === 'n') void (e.shiftKey ? this.newFolder() : this.newFile());
        else return;
      }
    }
    e.preventDefault();
  }

  // ------------------------------------------------------------ file operations

  private selectedEntries(): DirEntry[] {
    return this.view.filter((e) => e.name !== UP && this.selected.has(e.name));
  }

  private usable(list: DirEntry[]): DirEntry[] {
    if (list.some((e) => e.name_lossy)) toast('The name contains bytes that are not UTF-8: operations on such a file are unavailable.');
    return list.filter((e) => !e.name_lossy);
  }

  /** Run a request and show a readable error. Returns the result or null. */
  /** A lost connection is re-established on the next operation, then the operation is repeated once. */
  private async retrying<T>(fn: () => Promise<T>): Promise<T> {
    try {
      return await fn();
    } catch (e) {
      if (e instanceof ApiError && (e.code === 'connection_lost' || e.code === 'no_session') && this.hooks.onReconnect && (await this.hooks.onReconnect())) return await fn();
      throw e;
    }
  }

  private async run<T>(fn: () => Promise<T>): Promise<T | null> {
    try {
      return await this.retrying(fn);
    } catch (e) {
      if (e instanceof ApiError && (e.code === 'connection_lost' || e.code === 'no_session')) this.showLost();
      else toast(e instanceof ApiError ? e.message : String(e), e instanceof ApiError ? e.code : undefined);
      return null;
    } finally {
      this.body.focus(); // keyboard shortcuts keep working after a dialog
    }
  }

  private report(verb: string, r: BatchResult): void {
    if (!r.errors.length) return;
    const first = r.errors[0];
    toast(`${verb}: errors ${r.errors.length} of ${r.done.length + r.errors.length}. “${first.path.split('/').pop()}”: ${first.message}`, r.errors.map((e) => `${e.path}: ${e.message}`).join('\n'));
  }

  async newFolder(): Promise<void> {
    const name = await promptDialog('New folder', 'Folder name', 'New folder');
    if (!name) return;
    const r = await this.run(() => api.fs<{ path: string }>(this.sessionId, 'mkdir', { parent: this.path, name: name.trim() }));
    if (r) await this.reloadSelect(r.path);
  }

  async newFile(): Promise<void> {
    const name = await promptDialog('New file', 'File name', 'new file.txt', 'Create', [0, 'new file'.length]);
    if (!name) return;
    const n = name.trim();
    if (!n || n.includes('/') || n === '.' || n === '..') return void toast('Invalid file name.');
    // the file is created by the first save in the editor, not here
    this.hooks.onNewFile?.(this.join(n));
  }

  async renameSelected(): Promise<void> {
    const [e] = this.usable(this.selectedEntries());
    if (!e) return;
    const dot = e.kind === 'file' ? e.name.lastIndexOf('.') : -1;
    const name = await promptDialog('Rename', 'New name', e.name, 'Rename', [0, dot > 0 ? dot : e.name.length]);
    if (!name || name === e.name) return;
    const r = await this.run(() => api.fs<{ path: string }>(this.sessionId, 'rename', { path: this.join(e.name), name: name.trim() }));
    if (r) await this.reloadSelect(r.path);
  }

  async deleteSelected(): Promise<void> {
    const list = this.usable(this.selectedEntries());
    if (!list.length) return;
    const paths = list.map((e) => this.join(e.name));
    // count first: what exactly is about to disappear
    const c = await this.run(() => api.fs<{ files: number; dirs: number }>(this.sessionId, 'count', { paths }));
    if (!c) return;
    const examples = list.slice(0, 5).map((e) => paths[list.indexOf(e)]).join('\n') + (list.length > 5 ? `\n… plus ${list.length - 5}` : '');
    const stat = `${c.files} ${c.files === 1 ? 'file' : 'files'}${c.dirs ? `, ${c.dirs} ${c.dirs === 1 ? 'folder' : 'folders'}` : ''}`;
    const text = `Permanently deleted on the server: ${stat}.\n\n${examples}`;
    if (!(await confirmDialog('Delete?', text, 'Delete', true))) return;
    const r = await this.run(() => api.fs<BatchResult>(this.sessionId, 'delete', { paths }));
    if (r) {
      this.report('Deleting', r);
      await this.load(this.path);
    }
  }

  copySelected(cut: boolean): void {
    const list = this.usable(this.selectedEntries());
    if (!list.length) return;
    Explorer.clipboard = { session: this.sessionId, cut, dir: this.path, names: list.map((e) => e.name) };
    toast(`${cut ? 'Cut' : 'Copied'}: ${list.length}. Open the destination folder and paste (Ctrl+V).`);
  }

  async paste(dest = this.path): Promise<void> {
    const c = Explorer.clipboard;
    if (!c) return;
    if (c.session !== this.sessionId) {
      toast('Copying between different servers is not available yet.');
      return;
    }
    const paths = c.names.map((n) => (c.dir === '/' ? `/${n}` : `${c.dir}/${n}`));
    const r = await this.run(() => api.fs<BatchResult>(this.sessionId, c.cut ? 'move' : 'copy', { paths, dest }));
    if (!r) return;
    this.report(c.cut ? 'Moving' : 'Copying', r);
    if (c.cut && !r.errors.length) Explorer.clipboard = null;
    await this.load(this.path);
    const last = r.done[r.done.length - 1];
    if (last && last.startsWith(this.path === '/' ? '/' : this.path + '/')) this.selectName(last.slice(last.lastIndexOf('/') + 1));
  }

  async chmodSelected(): Promise<void> {
    const [e] = this.usable(this.selectedEntries());
    if (!e) return;
    const r0 = await chmodDialog(e.name, isDirLike(e), e.mode);
    if (!r0) return;
    const r = await this.run(() => api.fs<{ changed: number }>(this.sessionId, 'chmod', { path: this.join(e.name), mode: r0.mode, recursive: r0.recursive, files: r0.files, dirs: r0.dirs }));
    if (r) {
      if (r0.recursive) toast(`Permissions changed: ${r.changed}.`);
      await this.reloadSelect(this.join(e.name));
    }
  }

  private async copyPath(): Promise<void> {
    const [e] = this.selectedEntries();
    const p = e ? this.join(e.name) : this.path;
    try {
      await navigator.clipboard.writeText(p);
      toast('Path copied.');
    } catch {
      toast(p);
    }
  }

  private async reloadSelect(path: string): Promise<void> {
    await this.load(this.path);
    this.selectName(path.slice(path.lastIndexOf('/') + 1));
  }

  private selectName(name: string): void {
    const i = this.view.findIndex((e) => e.name === name);
    if (i < 0) return;
    this.selected.clear();
    this.selected.add(name);
    this.cursor = i;
    this.move(i, false);
  }

  // ------------------------------------------------------------ context menu

  private menu: HTMLElement | null = null;

  private closeMenu = (): void => {
    this.menu?.remove();
    this.menu = null;
    document.removeEventListener('mousedown', this.onDocDown, true);
  };

  private onDocDown = (e: Event): void => {
    if (this.menu && !this.menu.contains(e.target as Node)) this.closeMenu();
  };

  private showMenu(x: number, y: number, onRow: boolean): void {
    this.closeMenu();
    const sel = this.selectedEntries();
    const one = sel.length === 1;
    const hasClip = !!Explorer.clipboard;
    type Item = [label: string, key: string, ico: string, enabled: boolean, fn: () => void] | null;
    const items: Item[] = [];
    if (onRow) {
      const isFile = one && sel[0].kind !== 'dir' && !isDirLike(sel[0]);
      const dirOne = one && isDirLike(sel[0]);
      if (dirOne) items.push(['Open', 'Enter', 'folder', true, () => this.open(sel[0])]);
      if (isFile) items.push(['Edit', 'Enter', 'edit', true, () => this.open(sel[0])]);
      items.push(['Download…', '', 'download', true, () => void this.downloadSelected()]);
      if (dirOne) items.push(['Upload here…', '', 'upload', true, () => void this.uploadHere('files', this.join(sel[0].name))]);
      items.push(null);
      items.push(['Copy', 'Ctrl+C', 'copy', true, () => this.copySelected(false)]);
      items.push(['Cut', 'Ctrl+X', 'cut', true, () => this.copySelected(true)]);
      if (dirOne) items.push(['Paste into folder', '', 'paste', hasClip, () => void this.paste(this.join(sel[0].name))]);
      items.push(null);
      items.push(['Rename', 'F2', 'rename', one, () => void this.renameSelected()]);
      items.push(['Permissions…', '', 'key', one, () => void this.chmodSelected()]);
      items.push(['Copy path', '', 'link', one, () => void this.copyPath()]);
      items.push(['Properties', '', 'info', one, () => void propertiesDialog(this.join(sel[0].name), sel[0])]);
      items.push(null);
      items.push(['Delete', 'Del', 'trash', true, () => void this.deleteSelected()]);
    } else {
      items.push(['New folder', 'Ctrl+Shift+N', 'folder-plus', true, () => void this.newFolder()]);
      items.push(['New file', 'Ctrl+N', 'file-plus', true, () => void this.newFile()]);
      items.push(['Upload files…', '', 'upload', true, () => void this.uploadHere('files')]);
      items.push(['Upload folder…', '', 'upload', true, () => void this.uploadHere('folder')]);
      items.push(['Paste', 'Ctrl+V', 'paste', hasClip, () => void this.paste()]);
      items.push(null);
      items.push(['Copy path', '', 'link', true, () => void this.copyPath()]);
      items.push(['Refresh', 'F5', 'refresh', true, () => void this.load(this.path)]);
    }
    const menu = h('div', { class: 'ctx-menu', role: 'menu', 'data-testid': 'ctx-menu' });
    for (const it of items) {
      if (!it) {
        menu.append(h('div', { class: 'sep' }));
        continue;
      }
      const [label, key, ico, enabled, fn] = it;
      const b = h('button', { class: 'ctx-item', role: 'menuitem', 'data-testid': 'ctx-' + label, onclick: () => (this.closeMenu(), fn()) }, icon(ico, 14), h('span', {}, label), h('kbd', {}, key));
      if (!enabled) b.setAttribute('disabled', '');
      menu.append(b);
    }
    document.body.append(menu);
    const r = menu.getBoundingClientRect();
    menu.style.left = `${Math.max(4, Math.min(x, window.innerWidth - r.width - 4))}px`;
    menu.style.top = `${Math.max(4, Math.min(y, window.innerHeight - r.height - 4))}px`;
    this.menu = menu;
    document.addEventListener('mousedown', this.onDocDown, true);
    menu.addEventListener('keydown', (e) => e.key === 'Escape' && this.closeMenu());
  }

  // ------------------------------------------------------------ transfers

  private onTransferFinished(t: import('./api').TransferView): void {
    if (t.kind === 'upload' && (t.dest === this.path || t.dest.startsWith(this.path === '/' ? '/' : this.path + '/'))) void this.load(this.path);
    if (t.state === 'done') toast(`${t.kind === 'upload' ? 'Uploaded' : 'Downloaded'}: “${t.label}”${t.files > 1 ? ` (files: ${t.files})` : ''}.`);
  }

  async uploadHere(kind: 'files' | 'folder', dest = this.path): Promise<void> {
    const paths = await native.pick(kind);
    if (!paths.length) return;
    await this.upload(paths, dest);
  }

  async upload(paths: string[], dest: string): Promise<void> {
    const id = await startTransfer(this.sessionId, 'upload', paths, dest);
    if (id) this.xfer.kick();
  }

  async downloadPaths(remote: string[]): Promise<void> {
    const [dir] = await native.pick('folder');
    if (!dir) return;
    const id = await startTransfer(this.sessionId, 'download', remote, dir);
    if (id) this.xfer.kick();
  }

  async downloadSelected(): Promise<void> {
    const list = this.usable(this.selectedEntries());
    if (!list.length) return void toast('Select files or folders to download.');
    await this.downloadPaths(list.map((e) => this.join(e.name)));
  }

  // ------------------------------------------------------------ path bar

  private renderCrumbs(): void {
    clear(this.crumbs);
    const parts = this.path.split('/').filter(Boolean);
    let acc = '';
    const add = (label: string, path: string) => this.crumbs.append(h('button', { class: 'crumb', 'data-path': path, 'data-testid': 'crumb', title: path, onclick: () => void this.load(path) }, label));
    add('/', '/');
    for (const p of parts) {
      acc += '/' + p;
      this.crumbs.append(h('span', { class: 'crumb-sep' }, '›'));
      add(p, acc);
    }
  }

  // ------------------------------------------------------------ drag and drop

  /** Files dropped from Windows Explorer: Tauri reports the position, we find what is under the cursor. */
  private static initDrop(): void {
    if (Explorer.dropReady) return;
    Explorer.dropReady = true;
    const clearMarks = () => document.querySelectorAll('.drop-target').forEach((e) => e.classList.remove('drop-target'));
    const target = (x: number, y: number) => {
      const el = document.elementFromPoint(x, y);
      const ex = el?.closest('.explorer');
      const inst = ex ? Explorer.byEl.get(ex) : undefined;
      const mark = el?.closest('.vrow[data-droppable="1"], .crumb') as HTMLElement | null;
      return { inst, mark, ex };
    };
    void native.onFileDrop({
      enter: () => undefined,
      over: (x, y) => {
        clearMarks();
        const { mark, ex } = target(x, y);
        (mark ?? ex)?.classList.add('drop-target');
      },
      leave: clearMarks,
      drop: (paths, x, y) => {
        clearMarks();
        const { inst, mark } = target(x, y);
        if (!inst) return;
        void inst.upload(paths, mark ? inst.destOf(mark) : inst.path);
      },
    });
  }

  private destOf(el: HTMLElement): string {
    if (el.classList.contains('crumb')) return el.dataset.path ?? this.path;
    if (el.dataset.name === UP) return this.path.replace(/\/[^/]*$/, '') || '/';
    return this.join(el.dataset.name ?? '');
  }

  /** Internal drag (pointer events: HTML5 DnD does not work while Tauri's file drop is enabled). */
  private attachDrag(): void {
    this.body.addEventListener('pointerdown', (e) => {
      if (e.button !== 0 || e.ctrlKey || e.shiftKey) return;
      const row = (e.target as HTMLElement).closest('.vrow') as HTMLElement | null;
      if (!row) return;
      const name = row.dataset.name ?? '';
      const entry = this.view.find((x) => x.name === name);
      if (!entry || entry.name_lossy || name === UP) return;
      const x0 = e.clientX;
      const y0 = e.clientY;
      let ghost: HTMLElement | null = null;
      let names: string[] = [];
      let target: HTMLElement | null = null;
      let outside = false;
      const pid = e.pointerId;
      const clearTarget = () => target?.classList.remove('drop-target');
      const move = (ev: PointerEvent) => {
        if (!ghost) {
          if (Math.hypot(ev.clientX - x0, ev.clientY - y0) < 6) return;
          if (!this.selected.has(name)) {
            this.selected.clear();
            this.selected.add(name);
            this.render();
            this.updateStatus();
          }
          names = this.selectedEntries().filter((s) => !s.name_lossy).map((s) => s.name);
          ghost = h('div', { class: 'drag-ghost', 'data-testid': 'drag-ghost' }, names.length === 1 ? names[0] : `${names.length} objects`);
          document.body.append(ghost);
          try {
            this.body.setPointerCapture(pid);
          } catch {
            /* ignore */
          }
        }
        ghost.style.left = `${ev.clientX + 12}px`;
        ghost.style.top = `${ev.clientY + 12}px`;
        const r = this.el.getBoundingClientRect();
        const inside = ev.clientX >= r.left && ev.clientX <= r.right && ev.clientY >= r.top && ev.clientY <= r.bottom;
        clearTarget();
        target = null;
        if (inside) {
          const el = document.elementFromPoint(ev.clientX, ev.clientY)?.closest('.vrow[data-droppable="1"], .crumb') as HTMLElement | null;
          if (el && !(el.classList.contains('vrow') && names.includes(el.dataset.name ?? ''))) {
            target = el;
            el.classList.add('drop-target');
          }
        } else if (!outside && native.isTauri) {
          outside = true;
          cleanup();
          void this.beginDragOut(names);
        }
      };
      const cleanup = () => {
        document.removeEventListener('pointermove', move);
        document.removeEventListener('pointerup', up);
        document.removeEventListener('pointercancel', up);
        clearTarget();
        ghost?.remove();
        try {
          this.body.releasePointerCapture(pid);
        } catch {
          /* ignore */
        }
      };
      const up = () => {
        const wasDrag = !!ghost;
        const dest = target ? this.destOf(target) : null;
        cleanup();
        if (this.dragOutAbort) this.dragOutAbort.aborted = true;
        if (wasDrag) {
          this.suppressClick = true;
          setTimeout(() => (this.suppressClick = false), 0);
          if (dest) void this.moveInto(names, dest);
        }
      };
      document.addEventListener('pointermove', move);
      document.addEventListener('pointerup', up);
      document.addEventListener('pointercancel', up);
    });
  }

  /** Drag inside the panel onto a folder = move (with a confirmation unless switched off for this session). */
  private async moveInto(names: string[], dest: string): Promise<void> {
    if (!names.length) return;
    if (!Explorer.noAskMove.has(this.sessionId)) {
      const r = await modal<{ ok: boolean; remember: boolean }>((close) => {
        const cb = h('input', { type: 'checkbox', 'data-testid': 'move-remember' }) as HTMLInputElement;
        return h(
          'div',
          { role: 'dialog', 'data-testid': 'move-confirm' },
          h('h3', {}, 'Move?'),
          h('p', {}, `Move ${names.length === 1 ? `“${names[0]}”` : `objects (${names.length})`} to ${dest}?`),
          h('label', { class: 'check' }, cb, ' Do not ask again in this session'),
          h('div', { class: 'row end' }, h('button', { type: 'button', onclick: () => close({ ok: false, remember: false }), 'data-testid': 'move-cancel' }, 'Cancel'), h('button', { type: 'button', class: 'primary', onclick: () => close({ ok: true, remember: cb.checked }), 'data-testid': 'move-ok' }, 'Move')),
        );
      });
      if (!r?.ok) return;
      if (r.remember) Explorer.noAskMove.add(this.sessionId);
    }
    const paths = names.map((n) => this.join(n));
    const res = await this.run(() => api.fs<BatchResult>(this.sessionId, 'move', { paths, dest }));
    if (res) {
      this.report('Moving', res);
      await this.load(this.path);
    }
  }

  /** The pointer left the panel while dragging: stage the files locally, then start a native drag. */
  private async beginDragOut(names: string[]): Promise<void> {
    const paths = names.map((n) => this.join(n));
    const abort = { aborted: false };
    this.dragOutAbort = abort;
    const bar = h('div', { class: 'xf-fill running', style: 'width:0%' });
    const note = h('div', { class: 'muted' }, 'Preparing files…');
    const overlay = h('div', { class: 'stage-overlay', 'data-testid': 'stage-overlay' }, h('div', {}, h('strong', {}, 'Preparing… do not release the mouse button'), note, h('div', { class: 'xf-bar' }, bar)));
    document.body.append(overlay);
    let dest = '';
    try {
      const r = await transfers.start({ session_id: this.sessionId, kind: 'download', items: paths.map((p) => ({ path: p, action: 'replace' as const })), stage: true });
      dest = r.dest;
      const t = await TransferPanel.wait(
        r.id,
        (v) => {
          bar.style.width = `${v.total ? Math.min(100, Math.round((v.done / v.total) * 100)) : 0}%`;
          note.textContent = v.current || 'Preparing files…';
        },
        abort,
      );
      if (abort.aborted || t.state === 'cancelled') {
        await transfers.stageClear(dest).catch(() => undefined);
        return;
      }
      if (t.state === 'error') {
        toast(t.error ?? 'Could not prepare the files.');
        await transfers.stageClear(dest).catch(() => undefined);
        return;
      }
      const sep = dest.includes('\\') ? '\\' : '/';
      overlay.remove();
      await native.dragOut(names.map((n) => dest + sep + n));
    } catch (e) {
      toast(e instanceof ApiError ? e.message : String(e instanceof Error ? e.message : e));
    } finally {
      overlay.remove();
      this.dragOutAbort = null;
    }
  }
}
