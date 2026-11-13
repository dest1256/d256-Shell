// The monitor column to the right of a terminal: the indicators the user ticked, one below the other, for the target the
// user chose (this computer or any connected server). Both choices are remembered in settings.json.
import { clear, h } from './dom';
import { t } from './i18n';
import { icon } from './icons';
import { COLORS, drawChart, fmtBytes, fmtRate, LOCAL_TARGET, monitors, MonitorSource, Point, Sample, Series } from './monitor';
import { getSettings, onSettings, updateSettings } from './settings';
import { fmt1, fmtGhz, fmtPct, windowOf } from './taskmgr';

/** Every indicator, in the order they are shown. */
const ORDER = ['cpu', 'mem', 'gpu3d', 'cuda', 'vram', 'gputemp', 'cputemp', 'disk', 'net', 'procs'] as const;
type Ind = (typeof ORDER)[number];

const DEFAULTS: Record<'local' | 'server', Ind[]> = {
  local: ['cpu', 'mem', 'gpu3d', 'cuda', 'vram'],
  server: ['cpu', 'mem', 'net'],
};

const label = (k: Ind): string =>
  ({ cpu: 'CPU', mem: t('Memory'), gpu3d: t('GPU 3D'), cuda: 'CUDA (Compute)', vram: t('Video memory'), gputemp: t('GPU temperature'), cputemp: t('CPU temp.'), disk: t('Disks'), net: t('Network'), procs: t('Processes') })[k];

/** Which indicators this target can show (a server has no video card and no process list). */
function available(s: Sample | null): Set<Ind> {
  const out = new Set<Ind>(['cpu', 'mem', 'disk', 'net']);
  if (!s) return out;
  if (s.gpus?.length) {
    out.add('gpu3d');
    if (s.gpus.some((g) => 'Compute' in g.engines)) out.add('cuda'); // only the Windows counters tell the CUDA load
    out.add('vram');
    out.add('gputemp');
  }
  if (s.cpu_temp != null) out.add('cputemp');
  if (s.procs) out.add('procs');
  return out;
}

/** What is drawn for one indicator. */
interface Spec {
  id: string;
  title: string;
  value: string;
  color: string;
  series: Series[];
  max?: number;
  min?: number;
  rows?: string[][];
}

function specs(chosen: Ind[], s: Sample, pts: Point[]): Spec[] {
  const last = pts[pts.length - 1];
  const out: Spec[] = [];
  const multi = (s.gpus?.length ?? 0) > 1;
  for (const k of chosen) {
    if (k === 'cpu') out.push({ id: 'cpu', title: 'CPU', value: `${fmtPct(last?.cpu ?? null)} ${fmtGhz(s.cpu_mhz)}`.trim(), color: COLORS.cpu, series: [{ data: pts.map((p) => p.cpu), color: COLORS.cpu, fill: true }], max: 100 });
    else if (k === 'mem') {
      const m = s.mem;
      out.push({ id: 'mem', title: t('Memory'), value: m ? `${fmt1((m.total - m.available) / 1073741824)}/${fmt1(m.total / 1073741824)} GB (${fmtPct(last?.mem ?? null)})` : '—', color: COLORS.mem, series: [{ data: pts.map((p) => p.mem), color: COLORS.mem, fill: true }], max: 100 });
    } else if (k === 'disk') {
      (s.disks ?? []).forEach((d, i) => {
        const dp = last?.disks.find((x) => x.name === d.name);
        out.push({ id: `disk:${d.name}`, title: t('Disk {0} ({1})', i, d.name), value: fmtPct(dp?.util ?? null), color: '#5cc258', series: [{ data: pts.map((p) => p.disks.find((x) => x.name === d.name)?.util ?? null), color: '#5cc258', fill: true }], max: 100 });
      });
    } else if (k === 'net') {
      out.push({
        id: 'net',
        title: t('Network'),
        value: last ? `↓ ${fmtRate(last.rx)}  ↑ ${fmtRate(last.tx)}` : '…',
        color: COLORS.rx,
        series: [
          { data: pts.map((p) => p.rx), color: COLORS.rx, fill: true },
          { data: pts.map((p) => p.tx), color: COLORS.tx },
        ],
        min: 10 * 1024,
      });
    } else if (k === 'cputemp') {
      if (s.cpu_temp != null) out.push({ id: 'cputemp', title: t('CPU temp.'), value: `${Math.round(s.cpu_temp)} °C`, color: COLORS.temp, series: [{ data: pts.map((p) => p.cpuTemp), color: COLORS.temp, fill: true }], max: 100 });
    } else if (k === 'procs') {
      if (s.procs) out.push({ id: 'procs', title: t('Processes'), value: '', color: '#94a3b8', series: [], rows: s.procs.slice(0, 5).map((p) => [p.name, fmtPct(p.cpu), p.gpu == null ? '—' : fmtPct(p.gpu), p.vram == null ? '—' : fmtBytes(p.vram)]) });
    } else {
      for (const g of s.gpus ?? []) {
        const gp = last?.gpus.find((x) => x.id === g.id);
        const of = (f: (x: NonNullable<typeof gp>) => number | null) => pts.map((p) => { const q = p.gpus.find((x) => x.id === g.id); return q ? f(q) : null; });
        const name = (base: string) => (multi ? `${base} · ${g.name}` : base);
        if (k === 'gpu3d') out.push({ id: `gpu3d:${g.id}`, title: name(t('GPU 3D')), value: fmtPct(gp?.d3 ?? null), color: COLORS.gpu3d, series: [{ data: of((x) => x.d3), color: COLORS.gpu3d, fill: true }], max: 100 });
        else if (k === 'cuda') {
          if (gp?.compute != null || pts.some((p) => p.gpus.find((x) => x.id === g.id)?.compute != null)) out.push({ id: `cuda:${g.id}`, title: name('CUDA (Compute)'), value: fmtPct(gp?.compute ?? null), color: COLORS.cuda, series: [{ data: of((x) => x.compute), color: COLORS.cuda, fill: true }], max: 100 });
        } else if (k === 'vram') {
          if (g.vram_used !== null) out.push({ id: `vram:${g.id}`, title: name(t('Video memory')), value: g.vram_total ? `${fmtBytes(g.vram_used)} / ${fmtBytes(g.vram_total)}` : fmtBytes(g.vram_used), color: COLORS.vram, series: [{ data: of((x) => x.vram), color: COLORS.vram, fill: true }], max: 100 });
        } else if (k === 'gputemp') {
          if (g.temp !== null || g.power !== null) out.push({ id: `gputemp:${g.id}`, title: name(t('GPU temperature')), value: [g.temp !== null ? `${Math.round(g.temp)} °C` : '', g.power !== null ? `${Math.round(g.power)} W` : ''].filter(Boolean).join(' · '), color: COLORS.temp, series: [{ data: of((x) => x.temp), color: COLORS.temp, fill: true }], max: 100 });
        }
      }
    }
  }
  return out;
}

interface Tile {
  el: HTMLElement;
  title: HTMLElement;
  value: HTMLElement;
  canvas: HTMLCanvasElement | null;
  table: HTMLElement | null;
}

export class MonitorColumn {
  readonly el: HTMLElement;
  private sel = h('select', { 'data-testid': 'mon-col-target', title: t('What the column monitors') }) as HTMLSelectElement;
  private pop = h('div', { class: 'mc-pop hidden', 'data-testid': 'mon-col-popover' });
  private body = h('div', { class: 'mc-body', 'data-testid': 'mon-col-body' });
  private tiles = new Map<string, Tile>();
  private key = '';
  private watch: ReturnType<typeof monitors.watch> | null = null;
  private offSrc: (() => void) | null = null;
  private offs: (() => void)[] = [];
  private raf = 0;
  private lastPick = '';

  /** `own`: the target of the tab the column sits in; `active`: whether that tab is on screen. */
  constructor(
    private own: string,
    private active: () => boolean,
  ) {
    const pick = h('button', { class: 'mon-col-btn', type: 'button', title: t('Indicators on the monitor'), 'aria-label': t('Indicators on the monitor'), 'data-testid': 'mon-col-pick', onclick: () => this.togglePop() }, icon('settings', 13));
    this.el = h('div', { class: 'mon-colbox', 'data-testid': 'monitor-column' }, h('div', { class: 'mc-head' }, this.sel, pick), this.pop, this.body);
    this.sel.addEventListener('change', () => void updateSettings({ monitor_col_target: this.sel.value === this.own ? 'auto' : this.sel.value }));
    const away = (e: Event) => {
      if (!this.pop.classList.contains('hidden') && !this.pop.contains(e.target as Node) && !pick.contains(e.target as Node)) this.pop.classList.add('hidden');
    };
    document.addEventListener('pointerdown', away);
    this.offs.push(() => document.removeEventListener('pointerdown', away));
    this.offs.push(monitors.onTargets(() => this.sync()));
    this.offs.push(onSettings(() => this.sync()));
    this.sync();
  }

  dispose(): void {
    this.offSrc?.();
    this.watch?.release();
    this.offs.forEach((f) => f());
    cancelAnimationFrame(this.raf);
  }

  /** The target shown: the chosen one while it is connected, else the one of this tab. */
  private effective(): string {
    const c = getSettings().monitor_col_target;
    return c !== 'auto' && monitors.targets().some((x) => x.key === c) ? c : this.own;
  }

  private chosen(): Ind[] {
    const saved = getSettings().monitor_cols[this.key];
    const set = new Set(saved ?? DEFAULTS[this.key === LOCAL_TARGET ? 'local' : 'server']);
    return ORDER.filter((k) => set.has(k));
  }

  /** Targets, selected target and the indicator list may have changed. */
  private sync(): void {
    const key = this.effective();
    if (key !== this.key || !this.watch) {
      this.offSrc?.();
      this.watch?.release();
      this.key = key;
      this.watch = monitors.watch(key, this.active);
      this.offSrc = this.watch.src.subscribe(() => this.schedule());
      for (const tl of this.tiles.values()) tl.el.remove();
      this.tiles.clear();
    }
    const list = monitors.targets();
    if (!list.some((x) => x.key === key)) list.push({ key, name: key });
    clear(this.sel);
    for (const x of list) this.sel.append(h('option', { value: x.key }, x.name));
    this.sel.value = key;
    this.lastPick = '';
    this.fillPop();
    this.schedule();
  }

  private togglePop(): void {
    this.pop.classList.toggle('hidden');
    this.fillPop();
  }

  private fillPop(): void {
    const src = this.watch?.src;
    const have = available(src?.last ?? null);
    const on = new Set(this.chosen());
    const sig = [...have].join() + '|' + [...on].join() + '|' + this.key;
    if (sig === this.lastPick) return;
    this.lastPick = sig;
    clear(this.pop);
    this.pop.append(h('div', { class: 'mc-pop-title' }, t('Indicators on the monitor')));
    for (const k of ORDER) {
      if (!have.has(k)) continue;
      const box = h('input', { type: 'checkbox', 'data-testid': `mon-pick-${k}` }) as HTMLInputElement;
      box.checked = on.has(k);
      box.addEventListener('change', () => {
        const next = new Set(this.chosen());
        if (box.checked) next.add(k);
        else next.delete(k);
        void updateSettings({ monitor_cols: { ...getSettings().monitor_cols, [this.key]: ORDER.filter((x) => next.has(x)) } });
      });
      this.pop.append(h('label', { class: 'mc-pick' }, box, h('span', {}, label(k))));
    }
    this.pop.append(
      h('button', { class: 'tool-sm', type: 'button', 'data-testid': 'mon-pick-reset', onclick: () => {
        const cols = { ...getSettings().monitor_cols };
        delete cols[this.key];
        void updateSettings({ monitor_cols: cols });
      } }, t('Default set')),
    );
  }

  private schedule(): void {
    if (this.raf) return;
    this.raf = requestAnimationFrame(() => {
      this.raf = 0;
      this.render();
    });
  }

  private tile(sp: Spec): Tile {
    let tl = this.tiles.get(sp.id);
    if (tl) return tl;
    const title = h('span', { class: 'mt-t' });
    const value = h('span', { class: 'mt-v' });
    const canvas = sp.rows ? null : (h('canvas', { class: 'mt-chart' }) as HTMLCanvasElement);
    const table = sp.rows ? h('table', { class: 'tm-procs', 'data-testid': 'mon-procs' }) : null;
    const el = h('div', { class: 'mt', 'data-testid': 'mon-tile', 'data-tile': sp.id }, h('div', { class: 'mt-head' }, title, value), canvas ?? table);
    tl = { el, title, value, canvas, table };
    this.tiles.set(sp.id, tl);
    return tl;
  }

  private render(): void {
    const src: MonitorSource | undefined = this.watch?.src;
    if (!src) return;
    this.fillPop();
    const s = src.last;
    if (!s) {
      for (const tl of this.tiles.values()) tl.el.remove();
      this.tiles.clear();
      if (!this.body.querySelector('.mt-wait')) this.body.append(h('div', { class: 'muted mt-wait', 'data-testid': 'mon-col-wait' }, src.error ?? t('Collecting data…')));
      else this.body.querySelector('.mt-wait')!.textContent = src.error ?? t('Collecting data…');
      return;
    }
    this.body.querySelector('.mt-wait')?.remove();
    const { pts, n } = windowOf(src.history, src.intervalSec);
    const list = specs(this.chosen(), s, pts);
    const keep = new Set(list.map((x) => x.id));
    for (const [id, tl] of this.tiles) {
      if (!keep.has(id)) {
        tl.el.remove();
        this.tiles.delete(id);
      }
    }
    this.body.querySelector('.mt-empty')?.remove();
    if (!list.length) this.body.append(h('div', { class: 'muted mt-empty' }, t('Nothing is chosen: use the button above to pick the indicators.')));
    list.forEach((sp, i) => {
      const tl = this.tile(sp);
      if (this.body.children[i] !== tl.el) this.body.insertBefore(tl.el, this.body.children[i] ?? null);
      tl.title.textContent = sp.title;
      tl.value.textContent = sp.value;
      tl.el.style.borderLeftColor = sp.color;
      if (tl.canvas) drawChart(tl.canvas, sp.series, { max: sp.max, min: sp.min, frame: sp.color, points: n, grid: true });
      if (tl.table && sp.rows) {
        clear(tl.table);
        tl.table.append(h('tr', {}, h('th', {}, t('Name')), h('th', {}, 'CPU'), h('th', {}, 'GPU'), h('th', {}, t('VRAM'))));
        for (const r of sp.rows) tl.table.append(h('tr', { 'data-testid': 'mon-proc' }, ...r.map((c) => h('td', {}, c))));
      }
    });
  }
}
