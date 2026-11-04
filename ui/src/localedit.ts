// Editors for local content: script files (own and linked) and notes (stored encrypted in the vault).
import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands';
import { bracketMatching, indentOnInput } from '@codemirror/language';
import { highlightSelectionMatches, openSearchPanel, search, searchKeymap } from '@codemirror/search';
import { EditorState, Extension } from '@codemirror/state';
import { drawSelection, EditorView, highlightActiveLine, highlightActiveLineGutter, keymap, lineNumbers } from '@codemirror/view';
import { api, ApiError, ScriptItem } from './api';
import { h, modal, toast } from './dom';
import { encodingTitle, ENCODINGS, supportsBom } from './encodings';
import { languageFor, pickEncoding, themeExtensions } from './editor';
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
  private encButton = h('button', { class: 'tool-sm', 'data-testid': 'sc-enc', title: t('File encoding: reread, or save in another one'), onclick: () => void this.chooseEncoding() }, 'UTF-8');

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
        h('span', { class: 'ed-dir muted' }, item.kind === 'file' ? item.path : item.kind === 'link' ? t('link') + ' · ' + item.path : t('my script') + ' · ' + t('encrypted')),
        h('span', { class: 'ed-spacer' }),
        h('button', { class: 'tool-sm', 'data-testid': 'sc-find', title: t('Find and replace (Ctrl+F / Ctrl+H)'), onclick: () => openSearchPanel(this.view) }, icon('search', 13)),
        h('button', { class: 'tool-sm', 'data-testid': 'sc-save', title: t('Save (Ctrl+S)'), onclick: () => void this.save() }, t('Save')),
        item.kind === 'file' ? null : h('button', { class: 'tool-sm primary', 'data-testid': 'sc-run', title: t('Run in a new terminal tab'), onclick: () => void this.run() }, icon('terminal', 13), ' ' + t('Run')),
      ),
      h('div', { class: 'ed-host', 'data-testid': 'sc-host' }),
      h('div', { class: 'ed-statusbar' }, this.status, h('span', { class: 'ed-spacer' }), this.info, this.encButton),
    );
  }

  /** Load the script; when its bytes do not decode, ask for an encoding. False: the user gave up or it failed. */
  async open(): Promise<boolean> {
    try {
      await this.load();
      return true;
    } catch (e) {
      if (e instanceof ApiError && e.code === 'bad_encoding') {
        const enc = await pickEncoding(e.message);
        if (!enc) return false;
        try {
          await this.load(enc);
          return true;
        } catch (e2) {
          toast(errText(e2), e2 instanceof ApiError ? e2.code : undefined);
          return false;
        }
      }
      toast(errText(e), e instanceof ApiError ? e.code : undefined);
      return false;
    }
  }

  async load(encoding?: string): Promise<void> {
    const r = await api.scriptRead(this.item.id, encoding);
    this.saved = r.text;
    this.eol = r.eol;
    this.bom = r.bom;
    this.encoding = r.encoding;
    const host = this.el.querySelector('.ed-host') as HTMLElement;
    this.view?.destroy();
    host.replaceChildren();
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
    this.showInfo();
    this.status.textContent = r.guessed ? t('The encoding was detected automatically: {0}. If the text is wrong, choose the encoding at the bottom right.', r.encoding) : '';
    this.dirty = false;
  }

  private showInfo(): void {
    this.info.textContent = `${this.bom ? t('BOM') + ' · ' : ''}${this.eol.toUpperCase()}`;
    this.encButton.textContent = this.encoding;
    this.encButton.title = encodingTitle(this.encoding);
  }

  /** Encoding menu: read the file again in another encoding, or just save it in another one (converting). */
  private chooseEncoding(): Promise<void> {
    return modal<void>((close) => {
      const sel = h('select', { 'data-testid': 'enc-select' }) as HTMLSelectElement;
      for (const [n, label] of ENCODINGS) sel.append(h('option', { value: n, selected: n === this.encoding }, label));
      const bom = h('input', { type: 'checkbox', 'data-testid': 'enc-bom' }) as HTMLInputElement;
      bom.checked = this.bom;
      const bomRow = h('label', { class: 'check' }, bom, ' ' + t('Write a BOM (byte order mark)'));
      const sync = () => (bomRow.style.display = supportsBom(sel.value) ? '' : 'none');
      sel.addEventListener('change', sync);
      sync();
      return h(
        'div',
        { role: 'dialog', 'data-testid': 'enc-dialog' },
        h('h3', {}, t('Encoding')),
        h('p', {}, t('Now: {0}. “Reread” decodes the file again (if the text is shown as garbage). “Save as” writes the current text in the chosen encoding: this converts the file.', encodingTitle(this.encoding))),
        sel,
        bomRow,
        h(
          'div',
          { class: 'row end wrap' },
          h('button', { type: 'button', onclick: () => close(null), 'data-testid': 'enc-cancel' }, t('Cancel')),
          h('button', { type: 'button', 'data-testid': 'enc-reread', onclick: async () => {
            if (this.dirty && !window.confirm(t('Unsaved changes will be lost. Reread the file?'))) return;
            close(null);
            await this.reread(sel.value);
          } }, t('Reread in this encoding')),
          h('button', { type: 'button', class: 'primary', 'data-testid': 'enc-save-as', onclick: () => {
            this.encoding = sel.value;
            this.bom = supportsBom(sel.value) ? bom.checked : false;
            this.showInfo();
            this.setDirty(true);
            this.status.textContent = t('The file will be saved in the encoding {0}', this.encoding);
            close(null);
          } }, t('Save as this')),
        ),
      );
    }).then(() => {
      setTimeout(() => this.view?.focus(), 0);
    });
  }

  /** Read the script again with a chosen encoding and replace the text. */
  private async reread(encoding: string): Promise<void> {
    const was = this.dirty;
    try {
      await this.load(encoding);
      if (was) this.hooks.onDirty(false);
      this.status.textContent = t('The file was reread in the encoding {0}', encoding);
    } catch (e) {
      toast(errText(e), e instanceof ApiError ? e.code : undefined);
    }
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
