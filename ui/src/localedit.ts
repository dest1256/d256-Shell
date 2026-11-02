// Editors for local content: script files (own and linked) and notes (stored encrypted in the vault).
import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands';
import { bracketMatching, indentOnInput } from '@codemirror/language';
import { highlightSelectionMatches, openSearchPanel, search, searchKeymap } from '@codemirror/search';
import { EditorState, Extension } from '@codemirror/state';
import { drawSelection, EditorView, highlightActiveLine, highlightActiveLineGutter, keymap, lineNumbers } from '@codemirror/view';
import { api, ApiError, ScriptItem } from './api';
import { h, toast } from './dom';
import { languageFor, themeExtensions } from './editor';
import { t } from './i18n';
import { icon } from './icons';
import { errText } from './util';

export interface LocalHooks {
  onDirty: (dirty: boolean) => void;
  /** The pane wants to be closed (after a delete). */
  onClose: () => void;
}

function makeView(host: HTMLElement, doc: string, extra: Extension[], onChange: () => void): EditorView {
  const state = EditorState.create({
    doc,
    extensions: [
      history(),
      drawSelection(),
      indentOnInput(),
      bracketMatching(),
      highlightSelectionMatches(),
      search({ top: true }),
      themeExtensions(),
      keymap.of([{ key: 'Mod-h', run: openSearchPanel, preventDefault: true }, indentWithTab, ...searchKeymap, ...historyKeymap, ...defaultKeymap]),
      EditorView.updateListener.of((u) => u.docChanged && onChange()),
      ...extra,
    ],
  });
  return new EditorView({ state, parent: host });
}

/** A script file in the built-in editor with a Run button. */
export class ScriptPane {
  readonly el: HTMLElement;
  dirty = false;
  private view!: EditorView;
  private saved = '';
  private eol: 'lf' | 'crlf' = 'lf';
  private bom = false;
  private encoding = 'UTF-8';
  private status = h('span', { 'data-testid': 'sc-status' }, '');
  private info = h('span', { class: 'ed-info', 'data-testid': 'sc-info' }, '');

  constructor(
    private item: ScriptItem,
    private hooks: LocalHooks,
    private onRun: (id: string) => void,
  ) {
    this.el = h(
      'div',
      { class: 'editor', 'data-testid': 'script-editor', 'data-id': item.id },
      h(
        'div',
        { class: 'ed-bar' },
        h('span', { class: 'ed-path', title: item.path }, item.name),
        h('span', { class: 'ed-dir muted' }, item.kind === 'link' ? t('link') + ' · ' + item.path : t('my script')),
        h('span', { class: 'ed-spacer' }),
        h('button', { class: 'tool-sm', 'data-testid': 'sc-find', title: t('Find and replace (Ctrl+F / Ctrl+H)'), onclick: () => openSearchPanel(this.view) }, icon('search', 13)),
        h('button', { class: 'tool-sm', 'data-testid': 'sc-save', title: t('Save (Ctrl+S)'), onclick: () => void this.save() }, t('Save')),
        h('button', { class: 'tool-sm primary', 'data-testid': 'sc-run', title: t('Run in a new terminal tab'), onclick: () => void this.run() }, icon('terminal', 13), ' ' + t('Run')),
      ),
      h('div', { class: 'ed-host', 'data-testid': 'sc-host' }),
      h('div', { class: 'ed-statusbar' }, this.status, h('span', { class: 'ed-spacer' }), this.info),
    );
  }

  async load(): Promise<void> {
    const r = await api.scriptRead(this.item.id);
    this.saved = r.text;
    this.eol = r.eol;
    this.bom = r.bom;
    this.encoding = r.encoding;
    const host = this.el.querySelector('.ed-host') as HTMLElement;
    this.view = makeView(
      host,
      r.text,
      [
        lineNumbers(),
        highlightActiveLineGutter(),
        highlightActiveLine(),
        languageFor(this.item.name),
        keymap.of([{ key: 'Mod-s', run: () => (void this.save(), true), preventDefault: true }]),
      ],
      () => this.setDirty(this.view.state.doc.toString() !== this.saved),
    );
    this.info.textContent = `${this.encoding}${this.bom ? ' ' + t('BOM') : ''} · ${this.eol.toUpperCase()}`;
  }

  focus(): void {
    this.view?.focus();
  }

  private setDirty(d: boolean): void {
    if (d === this.dirty) return;
    this.dirty = d;
    this.hooks.onDirty(d);
    if (d) this.status.textContent = t('Modified');
  }

  async save(): Promise<boolean> {
    const text = this.view.state.doc.toString();
    try {
      await api.scriptWrite({ id: this.item.id, text, eol: this.eol, bom: this.bom, encoding: this.encoding });
    } catch (e) {
      this.status.textContent = t('Not saved');
      toast(errText(e), e instanceof ApiError ? e.code : undefined);
      return false;
    }
    this.saved = text;
    this.setDirty(false);
    this.status.textContent = t('Saved');
    return true;
  }

  private async run(): Promise<void> {
    if (this.dirty && !(await this.save())) return;
    this.onRun(this.item.id);
  }
}

/** A note: title + text, saved to the encrypted vault a moment after the last change. */
export class NotePane {
  readonly el: HTMLElement;
  dirty = false;
  private view!: EditorView;
  private title = h('input', { type: 'text', class: 'note-title', 'data-testid': 'note-title', 'aria-label': t('Note title'), placeholder: t('Note title'), maxlength: '200' }) as HTMLInputElement;
  private status = h('span', { 'data-testid': 'note-status' }, '');
  private timer = 0;
  private busy: Promise<void> = Promise.resolve();

  constructor(
    public id: string | undefined,
    private hooks: LocalHooks & { onTitle: (title: string, id: string) => void },
    private onDelete: () => void,
  ) {
    this.el = h(
      'div',
      { class: 'editor note', 'data-testid': 'note-editor' },
      h(
        'div',
        { class: 'ed-bar' },
        icon('lock', 13),
        this.title,
        h('span', { class: 'ed-spacer' }),
        h('button', { class: 'tool-sm', 'data-testid': 'note-delete', title: t('Delete note'), onclick: () => this.onDelete() }, icon('trash', 13)),
      ),
      h('div', { class: 'ed-host', 'data-testid': 'note-host' }),
      h('div', { class: 'ed-statusbar' }, this.status, h('span', { class: 'ed-spacer' }), h('span', { class: 'muted' }, t('Encrypted in the vault'))),
    );
    this.title.addEventListener('input', () => this.touch());
  }

  async load(): Promise<void> {
    let text = '';
    if (this.id) {
      const n = await api.noteGet(this.id);
      this.title.value = n.title;
      text = n.text;
    }
    const host = this.el.querySelector('.ed-host') as HTMLElement;
    this.view = makeView(host, text, [EditorView.lineWrapping, keymap.of([{ key: 'Mod-s', run: () => (void this.flush(), true), preventDefault: true }])], () => this.touch());
    this.dirty = false;
  }

  focus(): void {
    (this.id ? this.view : this.title)?.focus();
  }

  private touch(): void {
    if (!this.dirty) {
      this.dirty = true;
      this.hooks.onDirty(true);
    }
    this.status.textContent = t('Modified');
    clearTimeout(this.timer);
    this.timer = window.setTimeout(() => void this.flush(), 700);
  }

  /** Save now (also used before the pane is closed). */
  async flush(): Promise<void> {
    clearTimeout(this.timer);
    this.busy = this.busy.then(async () => {
      if (!this.dirty) return;
      const title = this.title.value;
      const text = this.view.state.doc.toString();
      try {
        const r = await api.noteSave({ id: this.id, title, text });
        this.id = r.id;
        this.hooks.onTitle(title.trim() || t('Untitled'), r.id);
        if (this.view.state.doc.toString() === text && this.title.value === title) {
          this.dirty = false;
          this.hooks.onDirty(false);
          this.status.textContent = t('Saved');
        }
      } catch (e) {
        this.status.textContent = t('Not saved');
        toast(errText(e), e instanceof ApiError ? e.code : undefined);
      }
    });
    await this.busy;
  }

  dispose(): void {
    clearTimeout(this.timer);
    this.view?.destroy();
  }
}
