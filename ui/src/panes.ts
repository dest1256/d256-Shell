// Panes inside one SSH tab: "Терминал" plus one pane per open file, with a small strip on top
// (shown only while there is more than one pane). Unsaved files are marked with a dot.
import { clear, h } from './dom';
import { icon } from './icons';

interface Pane {
  id: string;
  title: string;
  view: HTMLElement;
  closable: boolean;
  dirty: boolean;
  onShow?: () => void;
  /** Return false to keep the pane open (e.g. the user cancelled the "unsaved changes" question). */
  beforeClose?: () => Promise<boolean>;
  dispose?: () => void;
}

export class PaneSet {
  readonly el: HTMLElement;
  private strip = h('div', { class: 'pane-strip', role: 'tablist', 'data-testid': 'pane-strip' });
  private stage = h('div', { class: 'pane-stage' });
  private panes: Pane[] = [];
  private activeId = '';

  constructor(
    /** Show the strip even with a single pane (the side column needs a title and a close button). */
    private alwaysStrip = false,
    /** Called when the last pane was closed. */
    private onEmpty?: () => void,
  ) {
    this.el = h('div', { class: 'panes' }, this.strip, this.stage);
    this.strip.style.display = 'none';
  }

  get count(): number {
    return this.panes.length;
  }

  /** Dispose every pane (the tab is closing). */
  disposeAll(): void {
    for (const p of this.panes) p.dispose?.();
    this.panes = [];
  }

  add(p: Omit<Pane, 'dirty'>): void {
    const pane: Pane = { ...p, dirty: false };
    pane.view.classList.add('pane');
    this.panes.push(pane);
    this.stage.append(pane.view);
    this.activate(pane.id);
  }

  has(id: string): boolean {
    return this.panes.some((p) => p.id === id);
  }

  activate(id: string): void {
    const p = this.panes.find((x) => x.id === id);
    if (!p) return;
    this.activeId = id;
    for (const q of this.panes) q.view.style.display = q.id === id ? '' : 'none';
    this.renderStrip();
    p.onShow?.();
  }

  get active(): string {
    return this.activeId;
  }

  setDirty(id: string, dirty: boolean): void {
    const p = this.panes.find((x) => x.id === id);
    if (!p || p.dirty === dirty) return;
    p.dirty = dirty;
    this.renderStrip();
  }

  anyDirty(): boolean {
    return this.panes.some((p) => p.dirty);
  }

  async close(id: string): Promise<void> {
    const p = this.panes.find((x) => x.id === id);
    if (!p || !p.closable) return;
    if (p.beforeClose && !(await p.beforeClose())) return;
    p.dispose?.();
    p.view.remove();
    this.panes = this.panes.filter((x) => x !== p);
    if (this.activeId === id) this.activate(this.panes[this.panes.length - 1]?.id ?? '');
    this.renderStrip();
    if (!this.panes.length) this.onEmpty?.();
  }

  private renderStrip(): void {
    clear(this.strip);
    this.strip.style.display = this.panes.length > 1 || (this.alwaysStrip && this.panes.length > 0) ? '' : 'none';
    for (const p of this.panes) {
      const b = h(
        'div',
        { class: 'pane-tab' + (p.id === this.activeId ? ' active' : ''), role: 'tab', 'data-testid': `pane-${p.id}`, 'data-dirty': p.dirty ? '1' : '0', onclick: () => this.activate(p.id) },
        p.id === 'term' || p.id.startsWith('dup:') ? icon('terminal', 13) : p.id === 'monitor' ? icon('activity', 13) : icon('file', 13),
        h('span', { class: 'pane-title' }, p.title),
        p.dirty ? h('span', { class: 'dirty-dot', title: 'Есть несохранённые изменения', 'data-testid': 'dirty-dot' }, '●') : null,
        p.closable
          ? h('button', { class: 'pane-x', title: 'Закрыть', 'data-testid': `pane-close-${p.id}`, onclick: (e) => (e.stopPropagation(), void this.close(p.id)) }, icon('close', 11))
          : null,
      );
      this.strip.append(b);
    }
  }
}
