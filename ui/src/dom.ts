// Tiny DOM helpers (no framework).
type Attrs = Record<string, string | boolean | ((e: Event) => void) | undefined>;

export function h(tag: string, attrs: Attrs = {}, ...kids: (Node | string | null | undefined)[]): HTMLElement {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (v === undefined || v === false) continue;
    if (typeof v === 'function') el.addEventListener(k.replace(/^on/, ''), v);
    else if (v === true) el.setAttribute(k, '');
    else el.setAttribute(k, v);
  }
  for (const k of kids) if (k != null) el.append(k);
  return el;
}

export function clear(el: Element): void {
  while (el.firstChild) el.removeChild(el.firstChild);
}

/** Modal dialog; resolves with the value passed to `close`. */
export function modal<T>(build: (close: (v: T | null) => void) => HTMLElement): Promise<T | null> {
  return new Promise((resolve) => {
    const back = h('div', { class: 'modal-back' });
    const close = (v: T | null) => {
      back.remove();
      resolve(v);
    };
    const box = build(close);
    box.classList.add('modal');
    back.append(box);
    back.addEventListener('keydown', (e) => {
      if ((e as KeyboardEvent).key === 'Escape') close(null);
    });
    document.body.append(back);
    (box.querySelector('input,button') as HTMLElement | null)?.focus();
  });
}

export function confirmDialog(title: string, text: string, okLabel = 'OK', danger = false): Promise<boolean> {
  return modal<boolean>((close) =>
    h(
      'div',
      { role: 'dialog', 'data-testid': 'confirm' },
      h('h3', {}, title),
      h('p', {}, text),
      h(
        'div',
        { class: 'row end' },
        h('button', { onclick: () => close(false), 'data-testid': 'confirm-cancel' }, 'Отмена'),
        h('button', { class: danger ? 'danger' : 'primary', onclick: () => close(true), 'data-testid': 'confirm-ok' }, okLabel),
      ),
    ),
  ).then((v) => v === true);
}

export function toast(msg: string, detail?: string): void {
  const t = h('div', { class: 'toast', role: 'alert' }, msg);
  if (detail) {
    const d = h('pre', { class: 'detail' }, detail);
    d.style.display = 'none';
    t.append(
      h('button', { class: 'link', onclick: () => (d.style.display = d.style.display === 'none' ? 'block' : 'none') }, 'Подробнее'),
      d,
    );
  }
  document.body.append(t);
  setTimeout(() => t.remove(), 8000);
}

/** Single-line text prompt; resolves with the entered text or null on cancel. */
export function promptDialog(title: string, label: string, value = '', okLabel = 'OK', select?: [number, number]): Promise<string | null> {
  return modal<string>((close) => {
    const input = h('input', { type: 'text', value, 'data-testid': 'prompt-input', spellcheck: 'false', autocomplete: 'off' }) as HTMLInputElement;
    queueMicrotask(() => {
      input.focus();
      if (select) input.setSelectionRange(select[0], select[1]);
      else input.select();
    });
    return h(
      'form',
      { role: 'dialog', 'data-testid': 'prompt', onsubmit: (e) => (e.preventDefault(), close(input.value)) },
      h('h3', {}, title),
      h('label', {}, label, input),
      h(
        'div',
        { class: 'row end' },
        h('button', { type: 'button', onclick: () => close(null), 'data-testid': 'prompt-cancel' }, 'Отмена'),
        h('button', { type: 'submit', class: 'primary', 'data-testid': 'prompt-ok' }, okLabel),
      ),
    );
  });
}
