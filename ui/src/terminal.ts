// Terminal view: xterm.js attached to a backend PTY session (system ssh).
import { SearchAddon } from '@xterm/addon-search';
import { SerializeAddon } from '@xterm/addon-serialize';
import { Unicode11Addon } from '@xterm/addon-unicode11';
import { WebLinksAddon } from '@xterm/addon-web-links';
import { WebglAddon } from '@xterm/addon-webgl';
import { FitAddon } from '@xterm/addon-fit';
import { Terminal } from '@xterm/xterm';
import '@xterm/xterm/css/xterm.css';
import { api, ApiError, inTauri, termStream } from './api';
import { watchPrompts } from './connect';
import { h, modal, toast } from './dom';
import { icon } from './icons';
import { schemeById } from './schemes';
import { getSettings, onSettings, updateSettings } from './settings';

export type TermState = 'busy' | 'ok' | 'off';

export interface TerminalHooks {
  /** Esc after the session ended: close the tab. */
  onClose: () => void;
  onState: (s: TermState) => void;
}

const enc = new TextEncoder();

async function openLink(uri: string): Promise<void> {
  if (inTauri) {
    const { openUrl } = await import('@tauri-apps/plugin-opener');
    await openUrl(uri);
  } else {
    window.open(uri, '_blank', 'noopener,noreferrer');
  }
}

export async function copyText(text: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    const ta = document.createElement('textarea');
    ta.value = text;
    document.body.append(ta);
    ta.select();
    document.execCommand('copy');
    ta.remove();
  }
}

/**
 * Confirmation for multi-line / very large pastes: they may execute as commands. The text is shown in an editable
 * box (remove a dangerous line, fix a command) and what is left is what gets pasted. null = cancelled.
 */
async function confirmPaste(text: string, lines: number): Promise<string | null> {
  const count = (t: string) => {
    const n = t.split(/\r\n|\r|\n/).length;
    const size = t.length > 1024 ? `${(t.length / 1024).toFixed(1)} КБ` : `${t.length} симв.`;
    return { n, size };
  };
  const r = await modal<string | null>((close) => {
    const ta = h('textarea', { class: 'paste-edit', 'data-testid': 'paste-text', spellcheck: 'false', rows: String(Math.min(14, Math.max(5, lines + 1))), wrap: 'off', 'aria-label': 'Вставляемый текст' }) as HTMLTextAreaElement;
    ta.value = text;
    const title = h('h3', {}, lines > 1 ? `Вставить ${lines} строк?` : 'Вставить большой фрагмент?');
    const warn = h('p', { class: 'warn', 'data-testid': 'paste-warning' }, `Вставленный текст может выполниться как команды (${count(text).size}). Его можно отредактировать.`);
    const done = () => close(ta.value.length ? ta.value : null);
    ta.addEventListener('input', () => {
      const c = count(ta.value);
      title.textContent = c.n > 1 ? `Вставить ${c.n} строк?` : 'Вставить текст?';
      warn.textContent = `Вставленный текст может выполниться как команды (${c.size}). Его можно отредактировать.`;
    });
    ta.addEventListener('keydown', (e) => {
      if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) (e.preventDefault(), done());
    });
    queueMicrotask(() => {
      ta.focus();
      ta.setSelectionRange(ta.value.length, ta.value.length);
    });
    return h(
      'div',
      { role: 'alertdialog', class: 'wide', 'data-testid': 'paste-confirm' },
      title,
      warn,
      ta,
      h('div', { class: 'row end wrap' }, h('span', { class: 'muted small paste-hint' }, 'Ctrl+Enter — вставить'), h('button', { onclick: () => close(null), 'data-testid': 'paste-cancel' }, 'Отмена'), h('button', { class: 'primary', onclick: done, 'data-testid': 'paste-ok' }, 'Вставить')),
    );
  });
  return r;
}

export class TerminalView {
  readonly el: HTMLElement;
  private host: HTMLElement;
  private prefill: string | null;
  /** Lines to type into the shell of a copy once it is ready (folder, virtualenv, foreground program). */
  private startup: string[] = [];
  private startupTimer = 0;
  private startedAt = 0;
  private lastOutput = 0;
  private typedByUser = false;
  /** Time of the last real key press or paste (see `onInput`). */
  private lastUserInput = -1000;
  /** Last virtualenv activation typed in this terminal (heuristic for an idle shell, see `track`). */
  envCommands: string[] = [];
  private line = '';
  private lineDirty = false;
  private searchBar: HTMLElement;
  private searchInput: HTMLInputElement;
  readonly term: Terminal;
  private fit = new FitAddon();
  private search = new SearchAddon();
  private serializer = new SerializeAddon();
  private termId: string | null = null;
  private stopStream: (() => void) | null = null;
  private stopPrompts: (() => void) | null = null;
  private exited = true;
  private starting = false;
  private disposed = false;
  private wheelAcc = 0;
  private pending: Uint8Array[] = [];
  private flushing = false;
  private raf = 0;
  private fontSave: number | undefined;
  private unsub: () => void;

  constructor(
    private sessionId: string,
    private serverId: string | undefined,
    private hooks: TerminalHooks,
    opts: { prefill?: string; testid?: string; startup?: string[]; env?: string[] } = {},
  ) {
    this.host = h('div', { class: 'term-host', 'data-testid': opts.testid ?? 'terminal' });
    this.prefill = opts.prefill ?? null;
    this.startup = opts.startup ?? [];
    this.envCommands = opts.env ?? [];
    const s = getSettings();
    this.term = new Terminal({
      allowProposedApi: true,
      fontFamily: s.font_family,
      fontSize: s.font_size,
      cursorStyle: s.cursor_style,
      cursorBlink: true,
      scrollback: s.scrollback,
      theme: schemeById(s.scheme).theme,
      macOptionIsMeta: true,
      convertEol: false,
    });
    this.term.loadAddon(this.fit);
    this.term.loadAddon(this.search);
    this.term.loadAddon(this.serializer);
    this.term.loadAddon(new Unicode11Addon());
    this.term.unicode.activeVersion = '11';
    this.term.loadAddon(new WebLinksAddon((_e, uri) => void openLink(uri)));

    this.searchInput = h('input', { type: 'text', placeholder: 'Поиск в выводе', 'data-testid': 'term-search-input', spellcheck: 'false' }) as HTMLInputElement;
    this.searchBar = h(
      'div',
      { class: 'term-search', 'data-testid': 'term-search' },
      this.searchInput,
      h('button', { class: 'icon-btn', title: 'Предыдущее (Shift+Enter)', onclick: () => this.search.findPrevious(this.searchInput.value) }, icon('up', 14)),
      h('button', { class: 'icon-btn', title: 'Следующее (Enter)', onclick: () => this.search.findNext(this.searchInput.value) }, icon('down', 14)),
      h('button', { class: 'icon-btn', title: 'Закрыть (Esc)', onclick: () => this.toggleSearch(false) }, icon('close', 12)),
    );
    this.searchBar.style.display = 'none';
    this.searchInput.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') (e.shiftKey ? this.search.findPrevious(this.searchInput.value) : this.search.findNext(this.searchInput.value), e.preventDefault());
      if (e.key === 'Escape') this.toggleSearch(false);
    });
    this.searchInput.addEventListener('input', () => this.search.findNext(this.searchInput.value, { incremental: true }));

    this.el = h('div', { class: 'term-wrap' }, this.host, this.searchBar);
    this.applyScheme();

    this.term.open(this.host);
    // WebGL renderer with automatic fallback to the DOM renderer (e.g. no GPU, headless).
    try {
      const gl = new WebglAddon();
      gl.onContextLoss(() => gl.dispose());
      this.term.loadAddon(gl);
    } catch {
      /* DOM renderer stays */
    }
    (this.host as HTMLElement & { __term?: Terminal }).__term = this.term; // handle for tests

    this.term.onData((d) => this.onInput(d));
    this.term.onBinary((d) => this.send(Uint8Array.from(d, (c) => c.charCodeAt(0))));
    this.term.onResize(({ cols, rows }) => {
      if (this.termId && !this.exited) void api.termResize(this.termId, cols, rows).catch(() => undefined);
    });
    this.term.attachCustomKeyEventHandler((e) => this.onKey(e));
    // text entered through an input method (no key events) is the user typing as well
    this.host.addEventListener('beforeinput', () => (this.lastUserInput = performance.now()), true);

    // Paste: intercept to warn about multi-line text. Capture phase, before xterm's own handler.
    this.host.addEventListener(
      'paste',
      (e) => {
        e.preventDefault();
        e.stopPropagation();
        void this.paste(e.clipboardData?.getData('text') ?? '');
      },
      true,
    );
    this.host.addEventListener('contextmenu', (e) => {
      if (getSettings().right_click !== 'copy-paste') return;
      e.preventDefault();
      const sel = this.term.getSelection();
      if (sel) {
        void copyText(sel);
        this.term.clearSelection();
      } else {
        void navigator.clipboard.readText().then((t) => this.paste(t)).catch(() => toast('Не удалось прочитать буфер обмена.'));
      }
    });
    this.host.addEventListener(
      'wheel',
      (e) => {
        if (e.ctrlKey) {
          e.preventDefault();
          this.zoom(e.deltaY < 0 ? 1 : -1);
          return;
        }
        // Full-screen programs with mouse tracking (btop, htop, mc): ConPTY + ssh.exe on Windows may drop
        // mouse reports, so by default the wheel is sent as arrow keys, which these programs always understand.
        const t = this.term;
        if (getSettings().wheel_mode !== 'arrows' || this.exited || t.modes.mouseTrackingMode === 'none' || t.buffer.active.type !== 'alternate') return;
        e.preventDefault();
        e.stopPropagation();
        this.wheelAcc += e.deltaMode === 0 ? e.deltaY / 40 : e.deltaY * 3;
        const steps = Math.trunc(this.wheelAcc);
        if (!steps) return;
        this.wheelAcc -= steps;
        const arrow = (steps < 0 ? 'A' : 'B');
        const seq = (t.modes.applicationCursorKeysMode ? '\x1bO' : '\x1b[') + arrow;
        this.send(enc.encode(seq.repeat(Math.min(Math.abs(steps), 10))));
      },
      { passive: false, capture: true },
    );
    new ResizeObserver(() => this.scheduleFit()).observe(this.host);
    this.unsub = onSettings(() => this.applySettings());
  }

  // ------------------------------------------------------------ lifecycle

  async start(): Promise<void> {
    if (this.starting || this.disposed) return;
    this.starting = true;
    this.hooks.onState('busy');
    try {
      this.fitNow();
      if (this.prefill) {
        // a copy of another terminal: its screen and scrollback first, then the new shell continues below
        this.term.write(this.prefill);
        this.prefill = null;
      }
      const r = await api.termOpen(this.sessionId, this.term.cols, this.term.rows);
      this.termId = r.term_id;
      this.exited = false;
      this.startedAt = Date.now();
      this.armStartup();
      // askpass may ask questions (2FA code, a second password) while ssh logs in
      this.stopPrompts?.();
      this.stopPrompts = watchPrompts(r.term_id, { serverId: this.serverId, maxMs: 120_000 });
      this.stopStream?.();
      this.stopStream = termStream(
        r.term_id,
        (b) => {
          this.term.write(b);
          this.hooks.onState('ok');
          this.lastOutput = Date.now();
          this.armStartup();
        },
        (code) => this.onExit(code),
      );
      this.hooks.onState('ok');
    } catch (e) {
      this.exited = true;
      this.hooks.onState('off');
      const msg = e instanceof ApiError ? e.message : String(e);
      this.term.write(`\r\n\x1b[31m${msg}\x1b[0m\r\n\x1b[90m[Enter — повторить, Esc — закрыть вкладку]\x1b[0m\r\n`);
    } finally {
      this.starting = false;
    }
  }

  private onExit(code: number): void {
    this.exited = true;
    this.stopPrompts?.();
    this.stopPrompts = null;
    this.hooks.onState('off');
    this.term.write(`\r\n\x1b[90m[Сеанс завершён, код ${code}. Enter — переподключиться, Esc — закрыть вкладку]\x1b[0m\r\n`);
  }

  /** The tab became visible: refit and focus. */
  /** Screen + scrollback as escape sequences (colours kept) to pre-fill a copy of this terminal. The alternate
   * screen and mouse modes are left out: a copy must not start inside a half-drawn full-screen program. */
  snapshot(): string {
    return this.serializer.serialize({ excludeAltBuffer: true, excludeModes: true });
  }

  show(): void {
    this.scheduleFit();
    setTimeout(() => this.term.focus(), 0);
  }

  dispose(): void {
    this.disposed = true;
    window.clearTimeout(this.startupTimer);
    this.unsub();
    this.stopStream?.();
    this.stopPrompts?.();
    if (this.termId) void api.termClose(this.termId).catch(() => undefined);
    this.term.dispose();
  }

  // ------------------------------------------------------------ input

  /** The id of the backend terminal (null before it is opened). */
  get id(): string | null {
    return this.termId;
  }

  /**
   * Follow what the user types, line by line, only to notice virtualenv activation (an idle shell keeps no trace
   * of it on the server). Anything that makes the real line uncertain (Tab completion, arrows, paste) is ignored.
   */
  private track(d: string): void {
    if (d.length > 1 && d.startsWith('\x1b')) {
      this.lineDirty = true;
      return;
    }
    for (const ch of d) {
      if (ch === '\r' || ch === '\n') {
        if (!this.lineDirty) this.noteEnvCommands(this.line.trim());
        this.line = '';
        this.lineDirty = false;
      } else if (ch === '\x7f' || ch === '\b') this.line = this.line.slice(0, -1);
      else if (ch === '\x15' || ch === '\x03' || ch === '\x04') {
        this.line = '';
        this.lineDirty = false;
      } else if (ch < ' ') this.lineDirty = true;
      else this.line += ch;
    }
  }

  /**
   * `cd ~/App && source .venv/bin/activate` in one line is common: look at every command of the line. A `cd` just
   * before the activation is kept with it, because the path of the activation script is usually relative to it.
   */
  private noteEnvCommands(line: string): void {
    let lastCd: string | null = null;
    for (const seg of line.split(/\s*(?:&&|;)\s*/)) {
      if (/^cd(\s|$)/.test(seg)) lastCd = seg;
      else if (/^(source|\.)\s+\S*activate\S*\s*$/.test(seg) || /^(conda|mamba|micromamba)\s+activate\b/.test(seg)) {
        this.envCommands = lastCd && !seg.startsWith('conda') ? [lastCd, seg] : [seg];
      } else if (/^(deactivate|(conda|mamba|micromamba)\s+deactivate)\s*$/.test(seg)) this.envCommands = [];
    }
  }

  /** Type the startup lines once the new shell has finished printing its banner and prompt. */
  private armStartup(): void {
    if (!this.startup.length) return;
    window.clearTimeout(this.startupTimer);
    const waited = Date.now() - this.startedAt;
    // quiet for 500 ms = the prompt is there; never wait longer than 12 s
    this.startupTimer = window.setTimeout(() => void this.runStartup(), waited > 12_000 ? 0 : 500);
  }

  /** Wait until the terminal printed nothing for `quietMs` (at most `maxMs`). */
  private async waitQuiet(quietMs: number, maxMs: number): Promise<void> {
    const t0 = Date.now();
    while (Date.now() - this.lastOutput < quietMs && Date.now() - t0 < maxMs) await new Promise((r) => setTimeout(r, 100));
  }

  private async runStartup(): Promise<void> {
    const lines = this.startup;
    this.startup = [];
    if (!lines.length || this.typedByUser || this.exited || this.disposed) return; // the user already took over: do not mix
    for (const l of lines) {
      // after `su -` or a slow profile the shell needs a moment before it can take the next line
      await this.waitQuiet(400, 6000);
      if (this.typedByUser || this.exited || this.disposed) return;
      this.send(enc.encode(l + '\r'));
      await new Promise((r) => setTimeout(r, 150)); // let the echo start, so that "quiet" is measured after it
    }
  }

  private onInput(d: string): void {
    if (this.exited) {
      if (this.starting) return;
      if (d === '\r') void this.start();
      else if (d === '\x1b') this.hooks.onClose();
      return;
    }
    // xterm also answers the shell's own queries (e.g. BusyBox asks for the cursor position at start) through
    // onData: those are not the user typing and must neither stop the startup commands nor be tracked as a line
    if (performance.now() - this.lastUserInput < 80) {
      this.typedByUser = true;
      this.track(d);
    }
    this.send(enc.encode(d));
  }

  /** Input is queued and sent strictly in order (IPC calls may otherwise overtake each other). */
  private send(b: Uint8Array): void {
    if (!this.termId || this.exited) return;
    this.pending.push(b);
    if (!this.flushing) void this.flush();
  }

  private async flush(): Promise<void> {
    this.flushing = true;
    try {
      while (this.pending.length && this.termId) {
        const parts = this.pending.splice(0);
        const all = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
        let o = 0;
        for (const p of parts) (all.set(p, o), (o += p.length));
        await api.termWrite(this.termId, all).catch(() => undefined);
      }
    } finally {
      this.flushing = false;
    }
  }

  async paste(text: string): Promise<void> {
    if (!text || this.exited) return;
    const s = getSettings();
    const lines = text.split(/\r\n|\r|\n/).length;
    if (lines >= s.paste_warn_lines || text.length > s.paste_warn_bytes) {
      const edited = await confirmPaste(text, lines);
      if (edited === null) {
        this.term.focus();
        return;
      }
      text = edited; // the user may have changed it
    }
    this.lastUserInput = performance.now();
    this.term.paste(text); // honours bracketed paste mode
    this.term.focus();
  }

  private onKey(e: KeyboardEvent): boolean {
    if (e.type !== 'keydown') return true;
    if (!['Shift', 'Control', 'Alt', 'Meta', 'CapsLock'].includes(e.key)) this.lastUserInput = performance.now();
    const k = e.key.toLowerCase();
    if (e.ctrlKey && e.shiftKey && !e.altKey) {
      if (k === 'c') {
        const sel = this.term.getSelection();
        if (sel) void copyText(sel);
        return false;
      }
      if (k === 'v') {
        void navigator.clipboard.readText().then((t) => this.paste(t)).catch(() => toast('Не удалось прочитать буфер обмена.'));
        return false;
      }
      if (k === 'f') {
        this.toggleSearch(true);
        return false;
      }
      // application shortcuts (handled by the shell): let them bubble
      if (['t', 'w', 'b', 'd'].includes(k)) return false;
    }
    if (e.ctrlKey && !e.shiftKey && !e.altKey) {
      if (e.key === '=' || e.key === '+') return (this.zoom(1), false);
      if (e.key === '-') return (this.zoom(-1), false);
      if (e.key === '0') return (this.zoomReset(), false);
      if (e.key === 'Tab') return false; // tab switching
      // Ctrl+C, Ctrl+W, Ctrl+B, Ctrl+T ... go to the server unchanged (readline, tmux, ^C)
    }
    return true;
  }

  // ------------------------------------------------------------ view

  private toggleSearch(on: boolean): void {
    this.searchBar.style.display = on ? 'flex' : 'none';
    if (on) {
      this.searchInput.focus();
      this.searchInput.select();
    } else {
      this.search.clearDecorations();
      this.term.focus();
    }
  }

  private zoom(delta: number): void {
    const n = Math.max(6, Math.min(48, this.term.options.fontSize! + delta));
    this.term.options.fontSize = n;
    this.scheduleFit();
    window.clearTimeout(this.fontSave);
    this.fontSave = window.setTimeout(() => void updateSettings({ font_size: n }).catch(() => undefined), 500);
  }

  private zoomReset(): void {
    this.term.options.fontSize = 14;
    this.scheduleFit();
    void updateSettings({ font_size: 14 }).catch(() => undefined);
  }

  private applyScheme(): void {
    const t = schemeById(getSettings().scheme).theme;
    this.el.style.background = t.background ?? '#000';
  }

  private applySettings(): void {
    const s = getSettings();
    const o = this.term.options;
    o.fontFamily = s.font_family;
    if (o.fontSize !== s.font_size) o.fontSize = s.font_size;
    o.cursorStyle = s.cursor_style;
    o.scrollback = s.scrollback;
    o.theme = schemeById(s.scheme).theme;
    this.applyScheme();
    this.scheduleFit();
  }

  private scheduleFit(): void {
    if (this.raf) return;
    this.raf = requestAnimationFrame(() => {
      this.raf = 0;
      this.fitNow();
    });
  }

  private fitNow(): void {
    if (this.disposed || this.host.clientWidth < 20 || this.host.clientHeight < 20) return; // hidden tab
    try {
      this.fit.fit();
    } catch {
      /* not laid out yet */
    }
  }

  /** Visible text of the whole buffer (tests and diagnostics). */
  bufferText(): string {
    const b = this.term.buffer.active;
    const out: string[] = [];
    for (let i = 0; i < b.length; i++) out.push(b.getLine(i)?.translateToString(true) ?? '');
    return out.join('\n');
  }
}

