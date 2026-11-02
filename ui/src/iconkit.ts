// Icons for sessions, terminals, scripts and notes: a built-in glyph or an image the user imported (kept in data/icons).
import { api, ApiError } from './api';
import { h, modal, toast } from './dom';
import { hasIcon, icon, PICKER_ICONS } from './icons';
import { t } from './i18n';
import { native } from './native';
import { getSettings, updateSettings } from './settings';
import { errText } from './util';

const cache = new Map<string, string>();

/** Key under which an icon choice is stored in settings.json: "server:<id>", "profile:<id>", "script:<id>", "note:<id>". */
export const iconKey = (kind: string, id: string): string => `${kind}:${id}`;

/** The icon of an item: its custom choice, else the default glyph. Image icons load asynchronously. */
export function iconFor(key: string, fallback: string, size = 16): HTMLElement | SVGElement {
  const v = getSettings().icons[key];
  if (!v) return icon(fallback, size);
  if (hasIcon(v)) return icon(v, size);
  const img = h('img', { class: 'custom-ico', width: String(size), height: String(size), alt: '', draggable: 'false' }) as HTMLImageElement;
  const hit = cache.get(v);
  if (hit) img.src = hit;
  else {
    api
      .iconGet(v)
      .then((r) => {
        cache.set(v, r.data_url);
        img.src = r.data_url;
      })
      .catch(() => img.replaceWith(icon(fallback, size)));
  }
  return img;
}

/** Choose an icon for an item. Resolves true when the choice changed (the caller re-renders). */
export async function pickIcon(key: string, fallback: string): Promise<boolean> {
  const current = getSettings().icons[key] ?? '';
  const res = await modal<string>((close) => {
    const grid = h('div', { class: 'icon-grid', 'data-testid': 'icon-grid' });
    for (const name of PICKER_ICONS) {
      grid.append(h('button', { class: 'icon-cell' + (name === current ? ' on' : ''), type: 'button', title: name, 'data-testid': `icon-${name}`, onclick: () => close(name) }, icon(name, 20)));
    }
    return h(
      'div',
      { role: 'dialog', 'data-testid': 'icon-dialog' },
      h('h3', {}, t('Choose an icon')),
      grid,
      h(
        'div',
        { class: 'row end' },
        h('button', { type: 'button', 'data-testid': 'icon-file', onclick: async () => {
          try {
            const picked = await native.pick('files');
            if (!picked.length) return;
            close((await api.iconImport(picked[0])).name);
          } catch (e) {
            toast(errText(e), e instanceof ApiError ? e.code : undefined);
          }
        } }, icon('image', 14), ' ' + t('Image file…')),
        h('button', { type: 'button', 'data-testid': 'icon-reset', onclick: () => close('') }, t('Default')),
        h('button', { type: 'button', onclick: () => close(null) }, t('Cancel')),
      ),
    );
  });
  if (res === null || res === current) return false;
  const icons = { ...getSettings().icons };
  if (res === '') delete icons[key];
  else icons[key] = res;
  await updateSettings({ icons });
  void fallback;
  return true;
}
