// Left-column sections next to "Sessions": Terminals (local shells), Scripts (own files and links), Notes (encrypted in the vault).
// Each section gives a sidebar list and a page of cards for the home tab; shell.ts decides what is shown.
import { api, ApiError, CustomProfile, NoteItem, ScriptItem, ShellProfile } from './api';
import { clear, confirmDialog, h, modal, promptDialog, toast } from './dom';
import { t } from './i18n';
import { iconFor, iconKey, pickIcon } from './iconkit';
import { icon } from './icons';
import { native } from './native';
import { getSettings, updateSettings } from './settings';
import { errText } from './util';

export interface SectionCtx {
  openTerminal: (p: ShellProfile) => void;
  openScript: (s: ScriptItem) => void;
  /** Run the script, or switch to its tab when it is already running; `copy` always starts a new run. */
  runScript: (s: ScriptItem, copy?: boolean) => void;
  /** The name shown for a script was changed: update open tabs. */
  retitle: (scriptId: string) => void;
  openNote: (id?: string) => void;
  /** Close tabs showing a deleted item (ids start with this prefix). */
  closeTabs: (prefix: string) => void;
  /** Re-render the home page. */
  refreshHome: () => void;
}

export interface Section {
  side: HTMLElement;
  /** Cards for the home tab. */
  home: () => HTMLElement;
  refresh: () => Promise<void>;
}

const fail = (e: unknown) => toast(errText(e), e instanceof ApiError ? e.code : undefined);

const iconBtn = (name: string, title: string, testid: string, fn: () => void) =>
  h('button', { class: 'icon-btn', title, 'aria-label': title, 'data-testid': testid, type: 'button', onclick: (e) => (e.stopPropagation(), fn()) }, icon(name, 14));

/** "a b 'c d'" -> ["a","b","c d"] */
export function splitArgs(s: string): string[] {
  const out: string[] = [];
  const re = /"([^"]*)"|'([^']*)'|(\S+)/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(s))) out.push(m[1] ?? m[2] ?? m[3]);
  return out;
}
const joinArgs = (a: string[]): string => a.map((x) => (/\s/.test(x) ? `"${x}"` : x)).join(' ');

// ------------------------------------------------------------------ terminals

export function createTerminals(ctx: SectionCtx): Section {
  let profiles: ShellProfile[] = [];
  const list = h('ul', { class: 'servers', 'data-testid': 'profile-list', role: 'listbox' });
  const grid = h('div', { class: 'cards', 'data-testid': 'profile-cards' });

  const startup = () => getSettings().startup_profile;
  const setStartup = async (id: string) => {
    await updateSettings({ startup_profile: startup() === id ? 'home' : id });
    render();
  };

  function render(): void {
    clear(list);
    clear(grid);
    if (!profiles.length) list.append(h('li', { class: 'empty' }, t('No shells were found on this computer. Add your own profile.')));
    for (const p of profiles) {
      const key = iconKey('profile', p.id);
      const star = startup() === p.id;
      list.append(
        h(
          'li',
          { class: 'srv', role: 'option', tabindex: '0', 'data-testid': 'profile-item', 'data-id': p.id, ondblclick: () => ctx.openTerminal(p), onkeydown: (e) => (e as KeyboardEvent).key === 'Enter' && ctx.openTerminal(p) },
          h('span', { class: 'srv-ico' }, iconFor(key, p.custom ? 'box' : 'terminal')),
          h('div', { class: 'grow' }, h('div', { class: 'srv-name' }, p.name), h('div', { class: 'sub' }, (star ? '★ ' + t('opens at start') + ' · ' : '') + p.program)),
          h(
            'div',
            { class: 'srv-actions' },
            iconBtn('star', star ? t('Do not open at start') : t('Open at start'), 'profile-star', () => void setStartup(p.id)),
            iconBtn('image', t('Change icon…'), 'profile-icon', () => void pickIcon(key, 'terminal').then((c) => c && (render(), ctx.refreshHome()))),
          ),
        ),
      );
      grid.append(
        h(
          'button',
          { class: 'card-srv', 'data-testid': 'profile-card', ondblclick: () => ctx.openTerminal(p) },
          h('span', { class: 'srv-ico big' }, iconFor(key, p.custom ? 'box' : 'terminal', 20)),
          h('span', { class: 'grow left' }, h('strong', {}, p.name), h('span', { class: 'sub' }, p.program)),
        ),
      );
    }
  }

  async function refresh(): Promise<void> {
    try {
      profiles = (await api.shellProfiles()).profiles;
    } catch (e) {
      fail(e);
    }
    render();
  }

  const side = h(
    'div',
    { class: 'side-section', 'data-testid': 'section-terminals' },
    h('div', { class: 'side-head' }, h('span', {}, t('Terminals'))),
    h('div', { class: 'side-tools' }, h('button', { class: 'tool-sm', 'data-testid': 'profiles-edit', type: 'button', onclick: () => void profilesDialog().then(async (c) => { if (c) { await refresh(); ctx.refreshHome(); } }) }, icon('settings', 13), ' ' + t('My profiles…'))),
    list,
  );
  const home = () =>
    h(
      'div',
      { 'data-testid': 'home-terminals' },
      h('div', { class: 'home-head' }, h('h1', {}, t('Terminals')), h('p', {}, t('Local shells: PowerShell, cmd, WSL, bash and your own programs open in tabs next to the SSH sessions.'))),
      grid,
    );
  return { side, home, refresh };
}

/** Add, edit and remove the user's own shell profiles. Resolves true when something was saved. */
async function profilesDialog(): Promise<boolean> {
  const rows: CustomProfile[] = getSettings().shell_profiles.map((p) => ({ ...p, args: [...p.args] }));
  const body = h('div', { class: 'prof-rows', 'data-testid': 'prof-rows' });
  const render = () => {
    clear(body);
    rows.forEach((r, i) => {
      const inp = (v: string, ph: string, id: string, on: (s: string) => void, cls = '') => {
        const el = h('input', { type: 'text', value: v, placeholder: ph, class: cls, 'data-testid': id, spellcheck: 'false' }) as HTMLInputElement;
        el.addEventListener('input', () => on(el.value));
        return el;
      };
      body.append(
        h(
          'div',
          { class: 'prof-row' },
          inp(r.name, t('Name'), `prof-name-${i}`, (s) => (r.name = s)),
          inp(r.program, t('Program (full path or name)'), `prof-prog-${i}`, (s) => (r.program = s), 'wide'),
          inp(joinArgs(r.args), t('Arguments'), `prof-args-${i}`, (s) => (r.args = splitArgs(s))),
          inp(r.cwd, t('Start folder'), `prof-cwd-${i}`, (s) => (r.cwd = s)),
          iconBtn('trash', t('Remove'), `prof-del-${i}`, () => (rows.splice(i, 1), render())),
        ),
      );
    });
    if (!rows.length) body.append(h('p', { class: 'muted' }, t('No profiles of your own yet.')));
  };
  render();
  const ok = await modal<boolean>((close) =>
    h(
      'div',
      { role: 'dialog', 'data-testid': 'profiles-dialog', style: 'min-width: 680px' },
      h('h3', {}, t('My shell profiles')),
      h('p', { class: 'muted' }, t('Any program that works in a console: python, a Docker shell, ssh to a jump host, a custom script.')),
      body,
      h(
        'div',
        { class: 'row end' },
        h('button', { type: 'button', class: 'left-auto', 'data-testid': 'prof-add', onclick: () => (rows.push({ id: Math.random().toString(36).slice(2, 10), name: '', program: '', args: [], cwd: '' }), render()) }, icon('plus', 14), ' ' + t('Add profile')),
        h('button', { type: 'button', onclick: () => close(null) }, t('Cancel')),
        h('button', { type: 'button', class: 'primary', 'data-testid': 'prof-save', onclick: () => close(true) }, t('Save')),
      ),
    ),
  );
  if (!ok) return false;
  const keep = rows.filter((r) => r.program.trim());
  await updateSettings({ shell_profiles: keep.map((r) => ({ ...r, name: r.name.trim() || r.program.trim(), program: r.program.trim() })) });
  return true;
}

// ------------------------------------------------------------------ scripts

const SCRIPT_TYPES = ['.ps1', '.bat', '.sh', '.cmd'];

export function createScripts(ctx: SectionCtx): Section {
  let scripts: ScriptItem[] = [];
  const list = h('ul', { class: 'servers', 'data-testid': 'script-list', role: 'listbox' });
  const grid = h('div', { class: 'cards', 'data-testid': 'script-cards' });

  async function addLinks(paths: string[]): Promise<void> {
    if (!paths.length) return;
    try {
      const r = await api.scriptLinkAdd(paths);
      if (r.skipped.length) toast(t('Only .bat, .cmd, .ps1 and .sh files can be added. Skipped: {0}', r.skipped.length), r.skipped.join('\n'));
      await refresh();
      ctx.refreshHome();
    } catch (e) {
      fail(e);
    }
  }

  async function newScript(): Promise<void> {
    const res = await modal<{ name: string }>((close) => {
      const name = h('input', { type: 'text', 'data-testid': 'sc-new-name', value: 'script', spellcheck: 'false', maxlength: '100' }) as HTMLInputElement;
      const ext = h('select', { 'data-testid': 'sc-new-ext' }, ...SCRIPT_TYPES.map((x) => h('option', { value: x }, x))) as HTMLSelectElement;
      const go = () => name.value.trim() && close({ name: name.value.trim() + ext.value });
      name.addEventListener('keydown', (e) => (e as KeyboardEvent).key === 'Enter' && go());
      return h(
        'form',
        { role: 'dialog', 'data-testid': 'sc-new-dialog', onsubmit: (e) => (e.preventDefault(), go()) },
        h('h3', {}, t('New script')),
        h('label', {}, t('File name'), h('div', { class: 'row' }, name, ext)),
        h('p', { class: 'muted' }, t('It is stored encrypted in the vault.')),
        h('div', { class: 'row end' }, h('button', { type: 'button', onclick: () => close(null) }, t('Cancel')), h('button', { type: 'submit', class: 'primary', 'data-testid': 'sc-new-ok' }, t('Create'))),
      );
    });
    if (!res) return;
    try {
      const r = await api.scriptCreate(res.name, '');
      await refresh();
      ctx.refreshHome();
      const item = scripts.find((s) => s.id === r.id);
      if (item) ctx.openScript(item);
    } catch (e) {
      fail(e);
    }
  }

  async function locate(s: ScriptItem): Promise<void> {
    const picked = await native.pick('files');
    if (!picked.length) return;
    try {
      await api.scriptLinkMove(s.path, picked[0]);
      ctx.closeTabs('script:' + s.id);
      await refresh();
      ctx.refreshHome();
    } catch (e) {
      fail(e);
    }
  }

  async function remove(s: ScriptItem): Promise<void> {
    if (s.kind === 'own' && !(await confirmDialog(t('Delete script?'), t('“{0}” will be deleted from the vault.', shown(s)), t('Delete'), true))) return;
    try {
      await api.scriptDelete(s.id);
      ctx.closeTabs('script:' + s.id);
      const icons = { ...getSettings().icons };
      delete icons[iconKey('script', s.id)];
      const names = { ...getSettings().script_names };
      delete names[s.id];
      await updateSettings({ icons, script_names: names });
      await refresh();
      ctx.refreshHome();
    } catch (e) {
      fail(e);
    }
  }

  const shown = (s: ScriptItem) => getSettings().script_names[s.id] || s.name;
  const label = (s: ScriptItem) => (s.kind === 'link' ? shown(s) + ' 🔗' : shown(s));
  const sub = (s: ScriptItem) => {
    if (!s.exists) return '⚠ ' + t('file not found');
    const own = s.kind === 'link' ? s.path : t('my script') + ' · ' + t('encrypted');
    return shown(s) !== s.name ? s.name + ' · ' + own : own;
  };

  async function rename(s: ScriptItem): Promise<void> {
    const v = await promptDialog(t('Rename'), t('Name'), shown(s), 'OK');
    if (v === null) return;
    const names = { ...getSettings().script_names };
    if (v.trim() && v.trim() !== s.name) names[s.id] = v.trim();
    else delete names[s.id]; // empty or the file name: back to the file name
    await updateSettings({ script_names: names });
    ctx.retitle(s.id);
    render();
    ctx.refreshHome();
  }

  function render(): void {
    clear(list);
    clear(grid);
    const own = scripts.filter((s) => s.kind === 'own');
    const links = scripts.filter((s) => s.kind === 'link');
    const group = (title: string) => h('li', { class: 'group', 'aria-hidden': 'true' }, title);
    if (own.length) list.append(group(t('Mine')));
    const item = (s: ScriptItem) => {
      const key = iconKey('script', s.id);
      const open = () => (s.exists ? ctx.runScript(s) : toast(t('The file was not found: {0}', s.path)));
      return h(
        'li',
        { class: 'srv' + (s.exists ? '' : ' missing'), role: 'option', tabindex: '0', 'data-testid': 'script-item', 'data-id': s.id, 'data-missing': s.exists ? '0' : '1', onclick: open, onkeydown: (e) => (e as KeyboardEvent).key === 'Enter' && open() },
        h('span', { class: 'srv-ico' }, iconFor(key, 'script')),
        h('div', { class: 'grow' }, h('div', { class: 'srv-name' }, label(s)), h('div', { class: 'sub' }, sub(s))),
        h(
          'div',
          { class: 'srv-actions' },
          s.exists ? iconBtn('edit', t('Edit the script'), 'script-edit', () => ctx.openScript(s)) : iconBtn('search', t('Locate the file…'), 'script-locate', () => void locate(s)),
          s.exists ? iconBtn('copy', t('Run a copy in a new tab'), 'script-run-copy', () => ctx.runScript(s, true)) : null,
          iconBtn('rename', t('Rename'), 'script-rename', () => void rename(s)),
          iconBtn('image', t('Change icon…'), 'script-icon', () => void pickIcon(key, 'script').then((c) => c && (render(), ctx.refreshHome()))),
          iconBtn('trash', s.kind === 'link' ? t('Remove from the list') : t('Delete'), 'script-delete', () => void remove(s)),
        ),
      );
    };
    own.forEach((s) => list.append(item(s)));
    if (links.length) list.append(group(t('Links: the files stay where they are')));
    links.forEach((s) => list.append(item(s)));
    if (!scripts.length) list.append(h('li', { class: 'empty' }, t('No scripts yet. Create one or drop a file here.')));
    for (const s of scripts) {
      const key = iconKey('script', s.id);
      grid.append(
        h(
          'button',
          { class: 'card-srv' + (s.exists ? '' : ' missing'), 'data-testid': 'script-card', onclick: () => (s.exists ? ctx.runScript(s) : toast(t('The file was not found: {0}', s.path))) },
          h('span', { class: 'srv-ico big' }, iconFor(key, 'script', 20)),
          h('span', { class: 'grow left' }, h('strong', {}, label(s)), h('span', { class: 'sub' }, sub(s))),
        ),
      );
    }
  }

  async function refresh(): Promise<void> {
    try {
      const r = await api.scriptsList();
      scripts = r.scripts;
      if (r.migrated > 0) toast(t('Scripts moved into the encrypted vault: {0}', r.migrated));
    } catch (e) {
      fail(e);
    }
    render();
  }

  const drop = h('div', { class: 'dropzone', 'data-testid': 'script-drop', 'data-dropzone': 'scripts' }, icon('download', 16), h('span', {}, t('Drop .bat / .ps1 / .sh files here')), h('small', {}, t('only a link is saved, the file is not copied')));
  const side = h(
    'div',
    { class: 'side-section', 'data-testid': 'section-scripts', 'data-dropzone': 'scripts' },
    h('div', { class: 'side-head' }, h('span', {}, t('Scripts'))),
    h(
      'div',
      { class: 'side-tools' },
      h('button', { class: 'tool-sm primary', 'data-testid': 'sc-new', type: 'button', onclick: () => void newScript() }, icon('plus', 13), ' ' + t('New script')),
      h('button', { class: 'tool-sm', 'data-testid': 'sc-link', type: 'button', title: t('Add a link to a script file'), onclick: async () => void addLinks(await native.pick('files')) }, icon('link', 13), ' ' + t('Add link…')),
    ),
    list,
    drop,
  );
  const home = () =>
    h(
      'div',
      { 'data-testid': 'home-scripts', 'data-dropzone': 'scripts' },
      h('div', { class: 'home-head' }, h('h1', {}, t('Scripts')), h('p', {}, t('Write a script here, or drop a file: only a link to it is stored. “Run” opens a terminal tab with the right shell.'))),
      grid,
    );

  // dropping files from the file manager onto the section adds links (the desktop app only: a browser gives no paths)
  void native.onFileDrop({
    enter: (x, y) => drop.classList.toggle('over', !!document.elementFromPoint(x, y)?.closest('[data-dropzone="scripts"]')),
    over: (x, y) => drop.classList.toggle('over', !!document.elementFromPoint(x, y)?.closest('[data-dropzone="scripts"]')),
    leave: () => drop.classList.remove('over'),
    drop: (paths, x, y) => {
      drop.classList.remove('over');
      if (document.elementFromPoint(x, y)?.closest('[data-dropzone="scripts"]')) void addLinks(paths);
    },
  });
  return { side, home, refresh };
}

// ------------------------------------------------------------------ notes

export function createNotes(ctx: SectionCtx): Section {
  let notes: NoteItem[] = [];
  const list = h('ul', { class: 'servers', 'data-testid': 'note-list', role: 'listbox' });
  const grid = h('div', { class: 'cards', 'data-testid': 'note-cards' });
  const when = (sec: number) => new Date(sec * 1000).toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' });

  async function remove(n: NoteItem): Promise<void> {
    if (!(await confirmDialog(t('Delete note?'), t('“{0}” will be deleted from the vault.', n.title), t('Delete'), true))) return;
    try {
      await api.noteDelete(n.id);
      ctx.closeTabs('note:' + n.id);
      await refresh();
      ctx.refreshHome();
    } catch (e) {
      fail(e);
    }
  }

  function render(): void {
    clear(list);
    clear(grid);
    if (!notes.length) list.append(h('li', { class: 'empty' }, t('No notes yet. They are stored encrypted in the vault.')));
    for (const n of notes) {
      const key = iconKey('note', n.id);
      list.append(
        h(
          'li',
          { class: 'srv', role: 'option', tabindex: '0', 'data-testid': 'note-item', 'data-id': n.id, onclick: () => ctx.openNote(n.id), onkeydown: (e) => (e as KeyboardEvent).key === 'Enter' && ctx.openNote(n.id) },
          h('span', { class: 'srv-ico' }, iconFor(key, 'note')),
          h('div', { class: 'grow' }, h('div', { class: 'srv-name' }, n.title), h('div', { class: 'sub' }, when(n.updated))),
          h(
            'div',
            { class: 'srv-actions' },
            iconBtn('image', t('Change icon…'), 'note-icon', () => void pickIcon(key, 'note').then((c) => c && (render(), ctx.refreshHome()))),
            iconBtn('trash', t('Delete'), 'note-item-delete', () => void remove(n)),
          ),
        ),
      );
      grid.append(
        h(
          'button',
          { class: 'card-srv', 'data-testid': 'note-card', onclick: () => ctx.openNote(n.id) },
          h('span', { class: 'srv-ico big' }, iconFor(key, 'note', 20)),
          h('span', { class: 'grow left' }, h('strong', {}, n.title), h('span', { class: 'sub' }, when(n.updated))),
        ),
      );
    }
  }

  async function refresh(): Promise<void> {
    try {
      notes = (await api.noteList()).notes;
    } catch (e) {
      // locked vault etc.: show an empty list, the lock screen handles the rest
      notes = [];
      if (!(e instanceof ApiError && e.code === 'locked')) fail(e);
    }
    render();
  }

  const side = h(
    'div',
    { class: 'side-section', 'data-testid': 'section-notes' },
    h('div', { class: 'side-head' }, h('span', {}, t('Notes')), h('span', { class: 'muted lock-note' }, icon('lock', 12), ' ' + t('encrypted'))),
    h('div', { class: 'side-tools' }, h('button', { class: 'tool-sm primary', 'data-testid': 'note-new', type: 'button', onclick: () => ctx.openNote() }, icon('plus', 13), ' ' + t('New note'))),
    list,
  );
  const home = () =>
    h(
      'div',
      { 'data-testid': 'home-notes' },
      h('div', { class: 'home-head' }, h('h1', {}, t('Notes')), h('p', {}, t('Notes are encrypted in the vault together with your passwords. Name them as you like.')), h('div', { class: 'row' }, h('button', { class: 'primary', 'data-testid': 'home-note-new', onclick: () => ctx.openNote() }, icon('plus', 14), t('New note')))),
      grid,
    );
  return { side, home, refresh };
}

