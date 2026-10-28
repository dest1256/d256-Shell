// Transfer queue UI: conflict questions before a transfer starts, and the progress panel at the bottom
// of the file explorer (queue, progress, speed, cancel, retry, errors).
import { ApiError, ConflictAction, TransferView, transfers } from './api';
import { clear, h, modal, toast } from './dom';
import { icon } from './icons';

function fmtSize(n: number): string {
  if (n < 1024) return `${n} Б`;
  const u = ['КБ', 'МБ', 'ГБ', 'ТБ'];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < u.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v >= 100 ? v.toFixed(0) : v.toFixed(1)} ${u[i]}`;
}

/** One question about an existing target. Resolves to the action and whether to use it for the rest. */
function askConflict(name: string, isDir: boolean, where: string): Promise<{ action: ConflictAction | 'cancel'; all: boolean }> {
  return modal<{ action: ConflictAction | 'cancel'; all: boolean }>((close) => {
    const all = h('input', { type: 'checkbox', 'data-testid': 'conflict-all' }) as HTMLInputElement;
    const b = (label: string, action: ConflictAction | 'cancel', cls = '', id = '') =>
      h('button', { type: 'button', class: cls, 'data-testid': id || `conflict-${action}`, onclick: () => close({ action, all: all.checked }) }, label);
    return h(
      'div',
      { role: 'dialog', 'data-testid': 'conflict-dialog' },
      h('h3', {}, isDir ? 'Папка уже существует' : 'Файл уже существует'),
      h('p', {}, `«${name}» уже есть ${where}.`),
      isDir ? h('p', { class: 'muted' }, 'Содержимое папок будет объединено; для файлов с одинаковыми именами действует выбранное действие.') : null,
      h('label', { class: 'check' }, all, ' Применить ко всем'),
      h('div', { class: 'row end wrap' }, b('Отмена', 'cancel'), b('Пропустить', 'skip'), b('Оставить оба', 'rename'), b(isDir ? 'Объединить и заменить' : 'Заменить', 'replace', 'primary')),
    );
  }).then((v) => v ?? { action: 'cancel', all: false });
}

/** Ask about every clash, then queue the transfer. Returns the job id or null when cancelled / nothing to do. */
export async function startTransfer(sessionId: string, kind: 'upload' | 'download', paths: string[], dest: string): Promise<string | null> {
  if (!paths.length) return null;
  let plan;
  try {
    plan = await transfers.plan(sessionId, kind, paths, dest);
  } catch (e) {
    toast(e instanceof ApiError ? e.message : String(e), e instanceof ApiError ? e.code : undefined);
    return null;
  }
  const items: { path: string; action: ConflictAction }[] = [];
  let remembered: ConflictAction | null = null;
  for (const it of plan) {
    let action: ConflictAction = 'skip';
    if (it.exists) {
      if (remembered) action = remembered;
      else {
        const r = await askConflict(it.name, it.is_dir, kind === 'upload' ? 'на сервере' : 'в выбранной папке');
        if (r.action === 'cancel') return null;
        action = r.action;
        if (r.all) remembered = action;
      }
    } else action = 'replace';
    if (action === 'skip' && it.exists) continue;
    items.push({ path: it.path, action });
  }
  if (!items.length) {
    toast('Всё пропущено: нечего передавать.');
    return null;
  }
  try {
    const r = await transfers.start({ session_id: sessionId, kind, items, dest });
    return r.id;
  } catch (e) {
    toast(e instanceof ApiError ? e.message : String(e), e instanceof ApiError ? e.code : undefined);
    return null;
  }
}

export interface PanelHooks {
  /** A transfer into / out of a folder finished: the explorer may want to refresh. */
  onFinished: (t: TransferView) => void;
  sessionIds: () => string[];
  /** The live session: a retried transfer must not use a dead one. */
  currentSession: () => string;
}

export class TransferPanel {
  readonly el: HTMLElement;
  private list = h('div', { class: 'xf-list', 'data-testid': 'xfer-list' });
  private head = h('div', { class: 'xf-head' });
  private timer = 0;
  private seen = new Map<string, string>();
  private asked = new Set<string>();
  private collapsed = false;

  constructor(private hooks: PanelHooks) {
    this.el = h('div', { class: 'xfer', 'data-testid': 'xfer-panel' }, this.head, this.list);
    this.el.style.display = 'none';
  }

  /** Begin polling (cheap: one call per tick, only while something is active or just finished). */
  kick(): void {
    if (this.timer) return;
    const tick = async () => {
      this.timer = 0;
      let active = false;
      try {
        const all = await transfers.list();
        const mine = all.filter((t) => this.hooks.sessionIds().includes(t.session_id));
        active = mine.some((t) => t.state === 'running' || t.state === 'queued');
        this.render(mine);
        for (const t of mine) {
          const was = this.seen.get(t.id);
          if (was !== t.state) {
            this.seen.set(t.id, t.state);
            if (t.state === 'done' || t.state === 'error' || t.state === 'cancelled') this.hooks.onFinished(t);
            if (t.state === 'error' && t.error_code === 'foreign_owner' && !this.asked.has(t.id)) {
              this.asked.add(t.id);
              void this.askForeign(t);
            }
          }
        }
      } catch {
        /* session gone */
      }
      if (active) this.timer = window.setTimeout(tick, 400);
    };
    this.timer = window.setTimeout(tick, 50);
  }

  private async askForeign(t: TransferView): Promise<void> {
    const choice = await modal<'in_place' | 'replace' | 'cancel'>((close) =>
      h(
        'div',
        { role: 'dialog', 'data-testid': 'foreign-dialog' },
        h('h3', {}, 'Файл принадлежит другому пользователю'),
        h('p', {}, `«${t.label}»: при обычной (атомарной) замене владелец файла изменится на вас. Можно записать прямо в существующий файл (не атомарно: при обрыве файл может повредиться) или заменить файл и сменить владельца.`),
        h(
          'div',
          { class: 'row end wrap' },
          h('button', { type: 'button', onclick: () => close('cancel'), 'data-testid': 'foreign-cancel' }, 'Отмена'),
          h('button', { type: 'button', onclick: () => close('replace'), 'data-testid': 'foreign-replace' }, 'Заменить'),
          h('button', { type: 'button', class: 'primary', onclick: () => close('in_place'), 'data-testid': 'foreign-inplace' }, 'Записать на месте'),
        ),
      ),
    );
    if (!choice || choice === 'cancel') return;
    try {
      await transfers.retry(t.id, { session_id: this.hooks.currentSession(), ...(choice === 'in_place' ? { in_place: true } : { allow_foreign: true }) });
      this.kick();
    } catch (e) {
      toast(e instanceof ApiError ? e.message : String(e));
    }
  }

  private render(jobs: TransferView[]): void {
    this.el.style.display = jobs.length ? '' : 'none';
    clear(this.head);
    const running = jobs.filter((j) => j.state === 'running' || j.state === 'queued').length;
    this.head.append(
      h('span', { class: 'xf-title' }, `Передачи${running ? ` · активных: ${running}` : ''}`),
      h('button', { class: 'icon-btn', title: 'Очистить завершённые', 'data-testid': 'xfer-clear', onclick: () => void transfers.clear().then(() => this.kick()) }, icon('trash', 14)),
      h('button', { class: 'icon-btn', title: this.collapsed ? 'Развернуть' : 'Свернуть', onclick: () => ((this.collapsed = !this.collapsed), this.render(jobs)) }, icon(this.collapsed ? 'up' : 'down', 14)),
    );
    this.list.style.display = this.collapsed ? 'none' : '';
    clear(this.list);
    for (const j of jobs) this.list.append(this.row(j));
  }

  private row(j: TransferView): HTMLElement {
    const pct = j.total > 0 ? Math.min(100, Math.round((j.done / j.total) * 100)) : j.state === 'done' ? 100 : 0;
    const arrow = j.kind === 'upload' ? '↑' : '↓';
    const stateText: Record<string, string> = { queued: 'в очереди', running: `${pct}% · ${fmtSize(j.speed)}/с`, done: 'готово', cancelled: 'отменено', error: 'ошибка' };
    const bar = h('div', { class: 'xf-bar' }, h('div', { class: 'xf-fill ' + j.state, style: `width:${pct}%` }));
    const btns: HTMLElement[] = [];
    if (j.state === 'running' || j.state === 'queued') {
      btns.push(h('button', { class: 'icon-btn', title: 'Отменить', 'data-testid': 'xfer-cancel', onclick: () => void transfers.cancel(j.id).then(() => this.kick()) }, icon('close', 13)));
    } else if (j.state === 'error' || j.state === 'cancelled') {
      btns.push(h('button', { class: 'icon-btn', title: 'Повторить', 'data-testid': 'xfer-retry', onclick: () => void transfers.retry(j.id, { session_id: this.hooks.currentSession() }).then(() => this.kick()).catch((e) => toast(String(e instanceof Error ? e.message : e))) }, icon('refresh', 13)));
    }
    const notes: HTMLElement[] = [];
    if (j.error) notes.push(h('div', { class: 'xf-err', 'data-testid': 'xfer-error' }, j.error));
    if (j.state === 'done' && j.skipped.length) {
      notes.push(h('div', { class: 'xf-note', title: j.skipped.join('\n'), 'data-testid': 'xfer-skipped' }, `Пропущено: ${j.skipped.length} (наведите, чтобы увидеть)`));
    }
    return h(
      'div',
      { class: 'xf-row', 'data-testid': 'xfer-row', 'data-state': j.state, 'data-kind': j.kind },
      h('div', { class: 'xf-top' }, h('span', { class: 'xf-name', title: j.label }, `${arrow} ${j.label}`), h('span', { class: 'xf-state' }, stateText[j.state] ?? j.state), ...btns),
      j.state === 'running' || j.state === 'queued' ? bar : null,
      j.state === 'running' && j.current ? h('div', { class: 'xf-note' }, `${j.current} · ${fmtSize(j.done)} из ${fmtSize(j.total)}`) : null,
      ...notes,
    );
  }

  /** Resolve when a given job reaches a final state (used by drag-out staging). */
  static async wait(id: string, onProgress?: (t: TransferView) => void, signal?: { aborted: boolean }): Promise<TransferView> {
    for (;;) {
      const all = await transfers.list();
      const t = all.find((x) => x.id === id);
      if (!t) throw new Error('Передача не найдена.');
      onProgress?.(t);
      if (t.state === 'done' || t.state === 'error' || t.state === 'cancelled') return t;
      if (signal?.aborted) {
        await transfers.cancel(id);
      }
      await new Promise((r) => setTimeout(r, 150));
    }
  }
}

