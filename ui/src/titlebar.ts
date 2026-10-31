// Windows Terminal style title bar: app mark, tabs, "+" button, caption buttons.
import { getCurrentWindow } from '@tauri-apps/api/window';
import { inTauri } from './api';
import { clear, h } from './dom';
import { icon } from './icons';

/** Linux (WebKitGTK) windows keep the native title bar and resize borders. */
export const NATIVE_FRAME = typeof navigator !== 'undefined' && /Linux/.test(navigator.userAgent) && !/Android/.test(navigator.userAgent);

export interface Tab {
  id: string;
  title: string;
  icon: string;
  closable?: boolean;
  /** Connection indicator shown on the tab: busy (connecting), ok, off (session ended). */
  state?: 'busy' | 'ok' | 'off';
}

export interface TabHandlers {
  onSelect: (id: string) => void;
  onAdd?: (() => void) | null;
  onClose?: (id: string) => void;
  onRename?: (id: string) => void;
}

export class Titlebar {
  readonly el: HTMLElement;
  private tabsEl = h('div', { class: 'tabs', role: 'tablist' });
  private addEl = h('button', { class: 'tab-add', title: 'New session (Ctrl+T)', 'data-testid': 'tab-add' }, icon('plus'));
  private onAdd: (() => void) | null = null;

  constructor() {
    this.addEl.addEventListener('click', () => this.onAdd?.());
    const mark = h('div', { class: 'brand', 'data-tauri-drag-region': true }, icon('terminal', 16), h('span', { 'data-tauri-drag-region': true }, 'd256 Shell'));
    this.el = h('div', { class: 'titlebar', 'data-tauri-drag-region': true }, mark, this.tabsEl, this.addEl, h('div', { class: 'drag', 'data-tauri-drag-region': true }));
    // own caption buttons only where the window has no native frame (Windows); Linux keeps the system title bar
    if (inTauri && !NATIVE_FRAME) this.el.append(this.captionButtons());
  }

  /** Replace the tab strip. Without `onAdd` the "+" button is hidden (e.g. on the unlock screen). */
  setTabs(tabs: Tab[], activeId: string, hd: TabHandlers): void {
    clear(this.tabsEl);
    for (const t of tabs) {
      const active = t.id === activeId;
      const el = h(
        'div',
        {
          class: 'tab' + (active ? ' active' : ''),
          role: 'tab',
          tabindex: '0',
          title: t.title,
          'aria-selected': String(active),
          'data-testid': `tab-${t.id}`,
          onclick: () => hd.onSelect(t.id),
          ondblclick: () => t.closable && hd.onRename?.(t.id),
          onkeydown: (e) => {
            const k = (e as KeyboardEvent).key;
            if (k === 'Enter' || k === ' ') hd.onSelect(t.id);
          },
          // middle click closes, like in Windows Terminal
          onauxclick: (e) => {
            if ((e as MouseEvent).button === 1 && t.closable) hd.onClose?.(t.id);
          },
        },
        icon(t.icon, 14),
        t.state ? h('span', { class: `dot ${t.state}`, title: { busy: 'Connecting…', ok: 'Connected', off: 'Disconnected' }[t.state], 'data-testid': `tab-state-${t.id}`, 'data-state': t.state }) : null,
        h('span', { class: 'tab-title' }, t.title),
      );
      if (t.closable) {
        el.append(
          h(
            'button',
            {
              class: 'tab-close',
              title: 'Close tab (Ctrl+W)',
              'aria-label': 'Close tab',
              'data-testid': `tab-close-${t.id}`,
              onclick: (e) => (e.stopPropagation(), hd.onClose?.(t.id)),
            },
            icon('close', 10),
          ),
        );
      }
      this.tabsEl.append(el);
    }
    this.onAdd = hd.onAdd ?? null;
    this.addEl.style.display = this.onAdd ? '' : 'none';
  }

  private captionButtons(): HTMLElement {
    const win = getCurrentWindow();
    const maxBtn = h('button', { class: 'cap', title: 'Maximize', onclick: () => void win.toggleMaximize() }, icon('max', 12));
    const sync = async () => {
      const m = await win.isMaximized().catch(() => false);
      maxBtn.replaceChildren(icon(m ? 'restore' : 'max', 12));
      maxBtn.title = m ? 'Restore' : 'Maximize';
    };
    void win.onResized(sync);
    void sync();
    return h(
      'div',
      { class: 'caption' },
      h('button', { class: 'cap', title: 'Minimize', onclick: () => void win.minimize() }, icon('min', 12)),
      maxBtn,
      h('button', { class: 'cap close', title: 'Close', onclick: () => void win.close() }, icon('close', 12)),
    );
  }
}
