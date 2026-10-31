// Built-in editor (CodeMirror 6). Ctrl+S writes back through the backend's safe save (conflict check,
// temp file + atomic replace). Line endings / BOM / final newline are restored exactly by the backend:
// the editor only ever sees "\n".
import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands';
import { css } from '@codemirror/lang-css';
import { html } from '@codemirror/lang-html';
import { javascript } from '@codemirror/lang-javascript';
import { json } from '@codemirror/lang-json';
import { markdown } from '@codemirror/lang-markdown';
import { php } from '@codemirror/lang-php';
import { python } from '@codemirror/lang-python';
import { sql } from '@codemirror/lang-sql';
import { xml } from '@codemirror/lang-xml';
import { yaml } from '@codemirror/lang-yaml';
import { bracketMatching, defaultHighlightStyle, HighlightStyle, indentOnInput, LanguageSupport, StreamLanguage, syntaxHighlighting } from '@codemirror/language';
import { dockerFile } from '@codemirror/legacy-modes/mode/dockerfile';
import { nginx } from '@codemirror/legacy-modes/mode/nginx';
import { properties } from '@codemirror/legacy-modes/mode/properties';
import { shell } from '@codemirror/legacy-modes/mode/shell';
import { toml } from '@codemirror/legacy-modes/mode/toml';
import { MergeView } from '@codemirror/merge';
import { highlightSelectionMatches, openSearchPanel, search, searchKeymap } from '@codemirror/search';
import { Compartment, EditorState, Extension, Text } from '@codemirror/state';
import { drawSelection, EditorView, highlightActiveLine, highlightActiveLineGutter, keymap, lineNumbers } from '@codemirror/view';
import { tags as t } from '@lezer/highlight';
import { ApiError, editor as api_, EditBase, EditOpened } from './api';
import { h, modal, toast } from './dom';
import { encodingTitle, ENCODINGS, supportsBom } from './encodings';
import { icon } from './icons';
import { schemeById } from './schemes';
import { getSettings } from './settings';
import { gotoLine } from './editor-goto';

/** Language by file name / extension (sh, conf/ini, nginx, yaml, json, toml, xml/html, js/ts, py, php, sql, Dockerfile, md, systemd units...). */
export function languageFor(path: string): Extension {
  const name = path.split('/').pop() ?? '';
  const lower = name.toLowerCase();
  const ext = lower.includes('.') ? lower.slice(lower.lastIndexOf('.') + 1) : '';
  const stream = (m: Parameters<typeof StreamLanguage.define>[0]) => new LanguageSupport(StreamLanguage.define(m));
  if (lower === 'dockerfile' || lower.startsWith('dockerfile.') || ext === 'dockerfile') return stream(dockerFile);
  if (lower.includes('nginx') || ext === 'nginx') return stream(nginx);
  if (['sh', 'bash', 'zsh', 'ksh', 'profile', 'bashrc', 'zshrc', 'bash_profile', 'bash_aliases'].includes(ext) || /^\.?(bashrc|zshrc|profile|bash_profile|bash_aliases)$/.test(lower)) return stream(shell);
  switch (ext) {
    case 'js': case 'mjs': case 'cjs': case 'jsx': return javascript({ jsx: true });
    case 'ts': case 'mts': case 'cts': return javascript({ typescript: true });
    case 'tsx': return javascript({ typescript: true, jsx: true });
    case 'json': case 'jsonc': return json();
    case 'yaml': case 'yml': return yaml();
    case 'toml': return stream(toml);
    case 'xml': case 'svg': case 'plist': case 'xsl': return xml();
    case 'html': case 'htm': case 'vue': return html();
    case 'css': case 'scss': return css();
    case 'py': return python();
    case 'php': return php();
    case 'sql': return sql();
    case 'md': case 'markdown': return markdown();
    case 'conf': case 'cfg': case 'ini': case 'properties': case 'env': case 'service': case 'socket': case 'timer': case 'mount': case 'target': case 'desktop': case 'rules':
      return stream(properties);
    default:
      return [];
  }
}

/** Colours follow the terminal colour scheme, so the editor looks like part of the terminal. */
function themeExtensions(): Extension {
  const th = schemeById(getSettings().scheme).theme;
  const bg = th.background ?? '#0c0c0c';
  const fg = th.foreground ?? '#cccccc';
  const sel = th.selectionBackground ?? 'rgba(96,205,255,.3)';
  const col = (k: keyof typeof th, d: string) => (th[k] as string | undefined) ?? d;
  const view = EditorView.theme(
    {
      '&': { color: fg, backgroundColor: bg, height: '100%' },
      '.cm-content': { caretColor: fg, fontFamily: getSettings().font_family, fontSize: `${getSettings().font_size}px` },
      '.cm-cursor, .cm-dropCursor': { borderLeftColor: fg },
      '&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection': { backgroundColor: sel },
      '.cm-gutters': { backgroundColor: bg, color: col('brightBlack', '#767676'), border: 'none' },
      '.cm-activeLine': { backgroundColor: 'rgba(255,255,255,.04)' },
      '.cm-activeLineGutter': { backgroundColor: 'rgba(255,255,255,.06)', color: fg },
      '.cm-panels': { backgroundColor: '#252525', color: '#ddd' },
      '.cm-panels input, .cm-panels button': { font: 'inherit', fontSize: '12px' },
      '.cm-searchMatch': { backgroundColor: 'rgba(255,200,0,.25)' },
    },
    { dark: true },
  );
  const hl = HighlightStyle.define([
    { tag: [t.keyword, t.operatorKeyword, t.modifier], color: col('brightMagenta', '#b4009e') },
    { tag: [t.string, t.special(t.string)], color: col('green', '#13a10e') },
    { tag: [t.number, t.bool, t.null], color: col('brightYellow', '#f9f1a5') },
    { tag: [t.comment, t.lineComment, t.blockComment], color: col('brightBlack', '#767676'), fontStyle: 'italic' },
    { tag: [t.function(t.variableName), t.function(t.propertyName), t.labelName], color: col('brightBlue', '#3b78ff') },
    { tag: [t.typeName, t.className, t.namespace], color: col('brightCyan', '#61d6d6') },
    { tag: [t.propertyName, t.attributeName], color: col('cyan', '#3a96dd') },
    { tag: [t.tagName, t.heading], color: col('brightRed', '#e74856'), fontWeight: 'bold' },
    { tag: [t.operator, t.punctuation, t.separator], color: fg },
    { tag: [t.link, t.url], color: col('blue', '#0037da'), textDecoration: 'underline' },
    { tag: t.invalid, color: col('red', '#c50f1f') },
  ]);
  return [view, syntaxHighlighting(hl), syntaxHighlighting(defaultHighlightStyle, { fallback: true })];
}

export interface EditorHooks {
  /** Current SFTP session id of the tab (changes after a reconnect). */
  session: () => string;
  /** Re-open the SFTP session after a lost connection; true when the session is usable again. */
  reconnect: () => Promise<boolean>;
  onDirty: (dirty: boolean) => void;
  onClose: () => void;
}

export class EditorPane {
  readonly el: HTMLElement;
  private view!: EditorView;
  private wrap = new Compartment();
  private savedDoc: Text = Text.empty;
  private base: EditBase | null = null;
  private bom = false;
  private encoding = 'UTF-8';
  private encButton = h('button', { class: 'tool-sm', 'data-testid': 'ed-enc', title: 'File encoding: reread, or save in another one', onclick: () => void this.chooseEncoding() }, 'UTF-8');
  private eol: 'lf' | 'crlf' | 'mixed' = 'lf';
  private chosenEol: 'lf' | 'crlf' | null = null;
  private isNew = false;
  private status = h('span', { 'data-testid': 'ed-status' }, '');
  private info = h('span', { class: 'ed-info', 'data-testid': 'ed-info' }, '');
  private saving = false;
  dirty = false;

  constructor(
    readonly path: string,
    private hooks: EditorHooks,
  ) {
    const name = path.split('/').pop() ?? path;
    const wrapBtn = h('button', { class: 'tool-sm', 'data-testid': 'ed-wrap', title: 'Wrap lines', onclick: () => this.toggleWrap(wrapBtn) }, 'Wrap');
    this.el = h(
      'div',
      { class: 'editor', 'data-testid': 'editor', 'data-path': path },
      h(
        'div',
        { class: 'ed-bar' },
        h('span', { class: 'ed-path', title: path }, name),
        h('span', { class: 'ed-dir muted' }, path.slice(0, path.length - name.length)),
        h('span', { class: 'ed-spacer' }),
        wrapBtn,
        h('button', { class: 'tool-sm', 'data-testid': 'ed-find', onclick: () => openSearchPanel(this.view), title: 'Find and replace (Ctrl+F / Ctrl+H)' }, icon('search', 13)),
        h('button', { class: 'tool-sm primary', 'data-testid': 'ed-save', onclick: () => void this.save(), title: 'Save to the server (Ctrl+S)' }, 'Save'),
      ),
      h('div', { class: 'ed-host', 'data-testid': 'ed-host' }),
      h('div', { class: 'ed-statusbar' }, this.status, h('span', { class: 'ed-spacer' }), this.info, this.encButton),
    );
  }

  /** Load the file (or prepare a new one). Throws ApiError for refusals (binary, too large, ...). */
  async load(isNew: boolean, encoding?: string): Promise<void> {
    const o = await api_.open(this.hooks.session(), this.path, isNew, encoding);
    this.apply(o);
    const host = this.el.querySelector('.ed-host') as HTMLElement;
    const state = this.makeState(o.text);
    this.view = new EditorView({ state, parent: host });
    this.savedDoc = state.doc;
    this.updateInfo();
    this.status.textContent = isNew ? 'New file: it will be created on the first save' : '';
    if (o.eol === 'mixed') this.status.textContent = 'The file has mixed line endings: when saving you will choose LF or CRLF.';
    else if (o.guessed) this.status.textContent = `The encoding was detected automatically: ${o.encoding}. If the text is wrong, choose the encoding at the bottom right.`;
  }

  private apply(o: EditOpened): void {
    this.isNew = o.new;
    this.base = o.base;
    this.bom = o.bom;
    this.eol = o.eol;
    this.encoding = o.encoding;
    this.encButton.textContent = o.encoding;
    this.chosenEol = null;
  }

  private makeState(doc: string): EditorState {
    return EditorState.create({
      doc,
      extensions: [
        lineNumbers(),
        highlightActiveLineGutter(),
        highlightActiveLine(),
        drawSelection(),
        history(),
        indentOnInput(),
        bracketMatching(),
        highlightSelectionMatches(),
        search({ top: true }),
        this.wrap.of([]),
        languageFor(this.path),
        themeExtensions(),
        keymap.of([
          { key: 'Mod-s', run: () => (void this.save(), true), preventDefault: true },
          { key: 'Mod-h', run: openSearchPanel, preventDefault: true },
          { key: 'Mod-g', run: gotoLine, preventDefault: true },
          indentWithTab,
          ...searchKeymap,
          ...historyKeymap,
          ...defaultKeymap,
        ]),
        EditorView.updateListener.of((u) => {
          if (u.docChanged) this.setDirty(!u.state.doc.eq(this.savedDoc));
          if (u.docChanged || u.selectionSet) this.updateInfo();
        }),
      ],
    });
  }

  focus(): void {
    this.view?.focus();
  }

  private toggleWrap(btn: HTMLElement): void {
    const on = !btn.classList.contains('on');
    btn.classList.toggle('on', on);
    this.view.dispatch({ effects: this.wrap.reconfigure(on ? EditorView.lineWrapping : []) });
  }

  private setDirty(d: boolean): void {
    if (d === this.dirty) return;
    this.dirty = d;
    this.hooks.onDirty(d);
    if (d) this.status.textContent = 'Modified';
  }

  private updateInfo(): void {
    const s = this.view.state;
    const line = s.doc.lineAt(s.selection.main.head);
    const eol = this.eol === 'mixed' ? (this.chosenEol ?? 'mixed').toString().toUpperCase() : this.eol.toUpperCase();
    this.info.textContent = `Line ${line.number}, column ${s.selection.main.head - line.from + 1} · ${this.encoding}${this.bom ? ' BOM' : ''} · ${eol}`;
  }

  text(): string {
    return this.view.state.doc.toString();
  }

  private async chooseEol(): Promise<'lf' | 'crlf' | null> {
    if (this.eol !== 'mixed') return this.eol;
    if (this.chosenEol) return this.chosenEol;
    const r = await modal<'lf' | 'crlf'>((close) =>
      h(
        'div',
        { role: 'dialog', 'data-testid': 'eol-dialog' },
        h('h3', {}, 'Mixed line endings'),
        h('p', {}, 'The file has both LF and CRLF. All lines will be saved in one style. Which one to choose?'),
        h('div', { class: 'row end' }, h('button', { onclick: () => close(null as never), 'data-testid': 'eol-cancel' }, 'Cancel'), h('button', { onclick: () => close('lf'), 'data-testid': 'eol-lf' }, 'LF (Linux)'), h('button', { class: 'primary', onclick: () => close('crlf'), 'data-testid': 'eol-crlf' }, 'CRLF (Windows)')),
      ),
    );
    if (r) this.chosenEol = r;
    return r;
  }

  /** Ctrl+S. Returns true when the file on the server now equals the editor content. */
  async save(opts: { force?: boolean; in_place?: boolean } = {}, retried = false): Promise<boolean> {
    if (this.saving) return false;
    const eol = await this.chooseEol();
    if (!eol) return false;
    this.saving = true;
    this.status.textContent = 'Saving…';
    const text = this.text();
    const doc = this.view.state.doc;
    try {
      const r = await api_.save(this.hooks.session(), { path: this.path, text, bom: this.bom, eol, encoding: this.encoding, base: this.base, force: opts.force, in_place: opts.in_place });
      this.base = r.base;
      this.isNew = false;
      this.savedDoc = doc;
      this.setDirty(!this.view.state.doc.eq(doc));
      this.status.textContent = 'Saved';
      this.el.dispatchEvent(new CustomEvent('saved', { bubbles: true }));
      return true;
    } catch (e) {
      this.saving = false;
      return await this.onSaveError(e, text, opts, retried);
    } finally {
      this.saving = false;
      setTimeout(() => this.view.focus(), 0); // dialogs steal the focus: Ctrl+S must keep working
    }
  }

  private async onSaveError(e: unknown, text: string, opts: { force?: boolean; in_place?: boolean }, retried: boolean): Promise<boolean> {
    if (!(e instanceof ApiError)) {
      this.fail(String(e instanceof Error ? e.message : e));
      return false;
    }
    switch (e.code) {
      case 'connection_lost':
      case 'no_session': {
        if (!retried && (await this.hooks.reconnect())) return this.save(opts, true);
        this.fail('Connection lost. The content is kept in the editor; reconnect and press Ctrl+S again.');
        return false;
      }
      case 'conflict': {
        const remote = (e.details?.remote_text as string | null | undefined) ?? null;
        const choice = await this.conflictDialog(remote, text);
        if (choice === 'force') return this.save({ ...opts, force: true }, retried);
        this.status.textContent = 'Not saved: the file changed on the server';
        return false;
      }
      case 'deleted': {
        const ok = await modal<boolean>((close) =>
          h(
            'div',
            { role: 'dialog', 'data-testid': 'deleted-dialog' },
            h('h3', {}, 'The file was deleted on the server'),
            h('p', {}, `“${this.path}” no longer exists. Create it again with the current content?`),
            h('div', { class: 'row end' }, h('button', { onclick: () => close(false), 'data-testid': 'deleted-cancel' }, 'Cancel'), h('button', { class: 'primary', onclick: () => close(true), 'data-testid': 'deleted-create' }, 'Create again')),
          ),
        );
        if (ok) return this.save({ ...opts, force: true }, retried);
        return false;
      }
      case 'foreign_owner':
      case 'dir_not_writable': {
        const ok = await modal<boolean>((close) =>
          h(
            'div',
            { role: 'dialog', 'data-testid': 'inplace-dialog' },
            h('h3', {}, e.code === 'foreign_owner' ? 'A file of another user' : 'The folder is not writable'),
            h('p', {}, e.message),
            h('div', { class: 'row end' }, h('button', { onclick: () => close(false), 'data-testid': 'inplace-cancel' }, 'Cancel'), h('button', { class: 'primary', onclick: () => close(true), 'data-testid': 'inplace-ok' }, 'Write in place')),
          ),
        );
        if (ok) return this.save({ ...opts, in_place: true }, retried);
        this.status.textContent = 'Not saved';
        return false;
      }
      default:
        this.fail(e.message, e.code);
        return false;
    }
  }

  private fail(msg: string, detail?: string): void {
    this.status.textContent = `Error: ${msg}`;
    toast(msg, detail);
  }

  /** "File changed on the server": show the differences, or overwrite anyway, or cancel. */
  private conflictDialog(remote: string | null, mine: string): Promise<'force' | 'cancel'> {
    return modal<'force' | 'cancel'>((close) => {
      const diffHost = h('div', { class: 'diff-host', 'data-testid': 'diff-host' });
      const showDiff = () => {
        if (diffHost.childElementCount) return;
        if (remote === null) {
          diffHost.textContent = 'The file on the server is too large to show differences.';
          return;
        }
        const common = [EditorView.editable.of(false), EditorState.readOnly.of(true), themeExtensions(), lineNumbers()];
        new MergeView({ a: { doc: remote, extensions: common }, b: { doc: mine, extensions: common }, parent: diffHost, highlightChanges: true, gutter: true });
      };
      const box = h(
        'div',
        { role: 'dialog', 'data-testid': 'conflict-save-dialog', class: 'wide' },
        h('h3', {}, 'The file changed on the server'),
        h('p', {}, 'Since the file was opened it has changed on the server. Left: the version on the server, right: yours.'),
        diffHost,
        h(
          'div',
          { class: 'row end wrap' },
          h('button', { type: 'button', onclick: () => close('cancel'), 'data-testid': 'cs-cancel' }, 'Cancel'),
          h('button', { type: 'button', onclick: showDiff, 'data-testid': 'cs-diff' }, 'Show differences'),
          h('button', { type: 'button', class: 'danger', onclick: () => close('force'), 'data-testid': 'cs-force' }, 'Overwrite anyway'),
        ),
      );
      return box;
    }).then((v) => v ?? 'cancel');
  }

  /** Encoding menu: re-read the file in another encoding, or just save it in another one (converting). */
  private chooseEncoding(): Promise<void> {
    return modal<void>((close) => {
      const sel = h('select', { 'data-testid': 'enc-select' }) as HTMLSelectElement;
      for (const [n, t] of ENCODINGS) sel.append(h('option', { value: n, selected: n === this.encoding }, t));
      const bom = h('input', { type: 'checkbox', 'data-testid': 'enc-bom' }) as HTMLInputElement;
      bom.checked = this.bom;
      const bomRow = h('label', { class: 'check' }, bom, ' Write a BOM (byte order mark)');
      const sync = () => (bomRow.style.display = supportsBom(sel.value) ? '' : 'none');
      sel.addEventListener('change', sync);
      sync();
      const reread = h(
        'button',
        {
          type: 'button',
          'data-testid': 'enc-reread',
          onclick: async () => {
            if (this.dirty && !window.confirm('Unsaved changes will be lost. Reread the file?')) return;
            close(null);
            await this.reread(sel.value);
          },
        },
        'Reread in this encoding',
      );
      if (this.isNew) reread.setAttribute('disabled', '');
      const saveAs = h(
        'button',
        {
          type: 'button',
          class: 'primary',
          'data-testid': 'enc-save-as',
          onclick: () => {
            this.encoding = sel.value;
            this.bom = supportsBom(sel.value) ? bom.checked : false;
            this.encButton.textContent = this.encoding;
            this.updateInfo();
            this.setDirty(true);
            this.status.textContent = `The file will be saved in the encoding ${this.encoding}`;
            close(null);
          },
        },
        'Save as this',
      );
      return h(
        'div',
        { role: 'dialog', 'data-testid': 'enc-dialog' },
        h('h3', {}, 'Encoding'),
        h('p', {}, `Now: ${encodingTitle(this.encoding)}. “Reread” decodes the file from the server again (if the text is shown as garbage). “Save as” writes the current text in the chosen encoding: this converts the file.`),
        sel,
        bomRow,
        h('div', { class: 'row end wrap' }, h('button', { type: 'button', onclick: () => close(null), 'data-testid': 'enc-cancel' }, 'Cancel'), reread, saveAs),
      );
    }).then(() => {
      setTimeout(() => this.view.focus(), 0);
    }); // Ctrl+S must work right after the dialog
  }

  /** Read the file again with a chosen encoding and replace the text. */
  private async reread(encoding: string): Promise<void> {
    try {
      const o = await api_.open(this.hooks.session(), this.path, false, encoding);
      this.apply(o);
      const state = this.makeState(o.text);
      this.view.setState(state);
      this.savedDoc = state.doc;
      this.setDirty(false);
      this.updateInfo();
      this.status.textContent = `The file was reread in the encoding ${o.encoding}`;
    } catch (e) {
      this.fail(e instanceof ApiError ? e.message : String(e));
    }
  }

  dispose(): void {
    this.view?.destroy();
  }
}

/** Ask for an encoding when a file cannot be decoded automatically. Resolves to the name, or null. */
export function pickEncoding(reason: string): Promise<string | null> {
  return modal<string | null>((close) => {
    const sel = h('select', { 'data-testid': 'enc-pick' }) as HTMLSelectElement;
    for (const [n, t] of ENCODINGS) sel.append(h('option', { value: n }, t));
    return h(
      'div',
      { role: 'dialog', 'data-testid': 'enc-pick-dialog' },
      h('h3', {}, 'Choose an encoding'),
      h('p', {}, reason),
      sel,
      h('div', { class: 'row end' }, h('button', { onclick: () => close(null), 'data-testid': 'enc-pick-cancel' }, 'Cancel'), h('button', { class: 'primary', onclick: () => close(sel.value), 'data-testid': 'enc-pick-ok' }, 'Open')),
    );
  });
}
