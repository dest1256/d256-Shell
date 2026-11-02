// Dialogs and helpers used by the explorer: permissions, properties, delete confirmation.
import { DirEntry } from './api';
import { h, modal } from './dom';
import { modeString } from './explorer';
import { t, locale } from './i18n';

export interface ChmodResult {
  mode: number;
  recursive: boolean;
  files: boolean;
  dirs: boolean;
}

/** Permissions dialog: rwx checkboxes, octal field, optional recursion (never touches symlinks). */
export function chmodDialog(name: string, isDir: boolean, current: number | null): Promise<ChmodResult | null> {
  return modal<ChmodResult>((close) => {
    let mode = current === null ? 0o644 : current & 0o7777;
    const octal = h('input', { type: 'text', value: mode.toString(8).padStart(3, '0'), 'data-testid': 'chmod-octal', maxlength: '4', spellcheck: 'false' }) as HTMLInputElement;
    const boxes: HTMLInputElement[] = [];
    const labels = [t('Read'), t('Write'), t('Running')];
    const who = [t('Owner'), t('Group'), t('Others')];
    const grid = h('div', { class: 'chmod-grid' }, h('span', {}));
    for (const l of labels) grid.append(h('span', { class: 'muted' }, l));
    for (let g = 0; g < 3; g++) {
      grid.append(h('span', { class: 'muted' }, who[g]));
      for (let b = 0; b < 3; b++) {
        const bit = 1 << (8 - (g * 3 + b));
        const cb = h('input', { type: 'checkbox', 'data-testid': `chmod-${g}${b}` }) as HTMLInputElement;
        cb.checked = (mode & bit) !== 0;
        cb.addEventListener('change', () => {
          mode = cb.checked ? mode | bit : mode & ~bit;
          octal.value = mode.toString(8).padStart(3, '0');
        });
        boxes.push(cb);
        grid.append(cb);
      }
    }
    octal.addEventListener('input', () => {
      if (!/^[0-7]{3,4}$/.test(octal.value)) return;
      mode = parseInt(octal.value, 8);
      boxes.forEach((cb, i) => (cb.checked = (mode & (1 << (8 - i))) !== 0));
    });
    const rec = h('input', { type: 'checkbox', 'data-testid': 'chmod-recursive' }) as HTMLInputElement;
    const kind = h('select', { 'data-testid': 'chmod-kind' }, h('option', { value: 'both' }, t('files and folders')), h('option', { value: 'files' }, 'files only'), h('option', { value: 'dirs' }, 'folders only')) as HTMLSelectElement;
    kind.disabled = true;
    rec.addEventListener('change', () => (kind.disabled = !rec.checked));
    return h(
      'form',
      {
        role: 'dialog',
        'data-testid': 'chmod-dialog',
        onsubmit: (e) => {
          e.preventDefault();
          if (!/^[0-7]{3,4}$/.test(octal.value)) return;
          const k = kind.value;
          close({ mode: parseInt(octal.value, 8), recursive: rec.checked, files: !rec.checked || k !== 'dirs', dirs: !rec.checked || k !== 'files' });
        },
      },
      h('h3', {}, t('Permissions: {0}', name)),
      grid,
      h('label', {}, t('Octal value'), octal),
      isDir
        ? h('div', { class: 'col' }, h('label', { class: 'check' }, rec, ' ' + t('Apply recursively (symbolic links are not touched)')), h('label', {}, t('Apply'), kind))
        : null,
      h('div', { class: 'row end' }, h('button', { type: 'button', onclick: () => close(null), 'data-testid': 'chmod-cancel' }, t('Cancel')), h('button', { type: 'submit', class: 'primary', 'data-testid': 'chmod-ok' }, t('Apply'))),
    );
  });
}

export function propertiesDialog(path: string, e: DirEntry): Promise<unknown> {
  const row = (k: string, v: string) => h('div', { class: 'prop-row' }, h('span', { class: 'muted' }, k), h('span', { class: 'sel' }, v));
  const kind = e.kind === 'dir' ? t('Folder') : e.kind === 'symlink' ? (e.target_is_dir ? t('Link to a folder') : t('Symbolic link')) : e.kind === 'file' ? t('File') : t('Special file');
  return modal<boolean>((close) =>
    h(
      'div',
      { role: 'dialog', 'data-testid': 'props-dialog' },
      h('h3', {}, t('Properties')),
      row(t('Name'), e.name),
      row(t('Path'), path),
      row(t('Type'), kind),
      row(t('Size'), e.kind === 'file' ? t('{0} bytes', e.size) : '—'),
      row(t('Permissions'), e.mode === null ? '—' : `${modeString(e.mode, e.kind)} (${(e.mode & 0o7777).toString(8)})`),
      row(t('Owner'), e.owner ?? (e.uid === null ? '—' : String(e.uid))),
      row(t('Group'), e.group ?? (e.gid === null ? '—' : String(e.gid))),
      row(t('Modified'), e.mtime === null ? '—' : new Date(e.mtime * 1000).toLocaleString(locale())),
      h('div', { class: 'row end' }, h('button', { class: 'primary', onclick: () => close(true), 'data-testid': 'props-close' }, t('Close'))),
    ),
  );
}
