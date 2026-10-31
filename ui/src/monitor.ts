// Mini task manager under the terminal: numbers first, then (after a gap) small graphs; a click opens a
// detailed pane with big graphs and the same numbers. The data is read from /proc over SFTP (nothing is
// executed on the server), so it needs SFTP and a Linux server.
import { api, ApiError } from './api';
import { clear, h } from './dom';
import { getSettings } from './settings';

export interface Sample {
  cpu: [number, number] | null;
  cores: [number, number][];
  mem: { total: number; available: number; free: number; cached: number; swap_total: number; swap_free: number } | null;
  load: { l1: number; l5: number; l15: number; running: number; procs: number } | null;
  uptime: number | null;
  net: { rx: number; tx: number } | null;
  /** Cumulative counters per physical disk: [reads, sectors read, ms reading, writes, sectors written, ms writing, ms doing I/O, in flight]. */
  disks?: { name: string; c: number[] }[] | null;
  ifaces?: { name: string; rx: number; tx: number }[] | null;
  /** Average current core frequency, MHz (null when the kernel does not report it). */
  cpu_mhz?: number | null;
  info?: {
    hostname: string | null;
    os: string | null;
    cpu_model: string | null;
    devices?: {
      disks: Record<string, { size: number | null; kind: string; model: string | null }>;
      ifaces: Record<string, { speed_mbit: number | null; state: string | null; mac: string | null }>;
    };
  };
}

/** One point of the history, already converted to percentages and rates. */
/** Load of one disk between two samples. */
export interface DiskPoint {
  name: string;
  /** Bytes per second. */
  r: number;
  w: number;
  /** Operations per second. */
  ri: number;
  wi: number;
  /** Share of time the disk was busy, 0..100. */
  util: number;
  queue: number;
  /** Average response time of one operation, ms. */
  resp: number;
}

/** Traffic of one network interface between two samples (bytes per second). */
export interface IfacePoint {
  name: string;
  rx: number;
  tx: number;
}

export interface Point {
  disks: DiskPoint[];
  ifaces: IfacePoint[];
  mhz: number | null;
  dr: number;
  dw: number;
  cpu: number | null;
  cores: number[];
  mem: number | null;
  swap: number | null;
  load: number | null;
  rx: number;
  tx: number;
}

/** History cap: a minute of graph at the fastest rate (100 ms) is 600 points. */
const MAX_POINTS = 700;

const pct = (busy: number, total: number): number => (total > 0 ? Math.max(0, Math.min(100, (busy / total) * 100)) : 0);

export function fmtBytes(n: number): string {
  const u = ['B', 'KB', 'MB', 'GB', 'TB'];
  let v = n;
  let i = 0;
  while (v >= 1024 && i < u.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toLocaleString('en-US', { maximumFractionDigits: v >= 100 || i === 0 ? 0 : 1 })} ${u[i]}`;
}

export function fmtRate(n: number): string {
  return `${fmtBytes(n)}/s`;
}

/** Bits per second like in the Windows task manager: 112 kbit/s, 20.7 Mbit/s. */
export function fmtBits(bytesPerSec: number): string {
  const bits = bytesPerSec * 8;
  const u = ['bit/s', 'kbit/s', 'Mbit/s', 'Gbit/s'];
  let v = bits;
  let i = 0;
  while (v >= 1000 && i < u.length - 1) {
    v /= 1000;
    i++;
  }
  return `${v.toLocaleString('en-US', { maximumFractionDigits: v >= 100 || i === 0 ? 0 : 1 })} ${u[i]}`;
}

/** d:hh:mm:ss, the uptime format of the task manager. */
export function fmtDuration(sec: number | null): string {
  if (sec === null) return '—';
  const p = (n: number) => String(Math.floor(n)).padStart(2, '0');
  return `${Math.floor(sec / 86400)}:${p((sec % 86400) / 3600)}:${p((sec % 3600) / 60)}:${p(sec % 60)}`;
}

export function fmtUptime(sec: number | null): string {
  if (sec === null) return '—';
  const d = Math.floor(sec / 86400);
  const hh = Math.floor((sec % 86400) / 3600);
  const m = Math.floor((sec % 3600) / 60);
  return d ? `${d} d ${hh} h` : hh ? `${hh} h ${m} min` : `${m} min`;
}

/** Polls the server and keeps the history. One per SSH tab; pauses while the tab is not visible. */
export class MonitorSource {
  history: Point[] = [];
  last: Sample | null = null;
  info: Sample['info'] = undefined;
  error: string | null = null;
  /** Total interval covered by the history, seconds. */
  private listeners = new Set<() => void>();
  private timer = 0;
  private prev: { s: Sample; at: number } | null = null;
  private running = false;
  private stopped = true;
  /** The user paused the graphs (no polling until resumed). */
  paused = false;

  constructor(
    private session: () => string,
    private active: () => boolean,
  ) {}

  subscribe(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  private emit(): void {
    for (const l of this.listeners) l();
  }

  get intervalSec(): number {
    return getSettings().monitor_interval_ms / 1000;
  }

  start(): void {
    if (!this.stopped) return;
    this.stopped = false;
    this.error = null;
    void this.tick();
  }

  /** Apply a new interval / pause at once instead of after the current wait. */
  restart(): void {
    window.clearTimeout(this.timer);
    if (!this.stopped) void this.tick();
    this.emit();
  }

  stop(): void {
    this.stopped = true;
    window.clearTimeout(this.timer);
  }

  private schedule(): void {
    if (this.stopped) return;
    this.timer = window.setTimeout(() => void this.tick(), Math.max(100, getSettings().monitor_interval_ms));
  }

  private missingDevices(): boolean {
    const dev = this.info?.devices;
    const l = this.last;
    if (!dev || !l) return false;
    return (l.disks ?? []).some((d) => !(d.name in dev.disks)) || (l.ifaces ?? []).some((i) => !(i.name in dev.ifaces));
  }

  private async tick(): Promise<void> {
    if (this.stopped || this.running) return;
    if (this.paused || !getSettings().monitor_enabled || !this.active() || document.hidden) return this.schedule();
    this.running = true;
    try {
      // device facts (disk size / model, link speed) are read with the first sample and again when a disk or an
      // interface shows up later (idle disks are hidden at first)
      const s = (await api.monitorSample(this.session(), !this.info || this.missingDevices())) as unknown as Sample;
      const at = performance.now();
      if (s.info) this.info = s.info;
      this.last = s;
      this.error = null;
      if (this.prev) this.history.push(this.toPoint(this.prev, { s, at }));
      if (this.history.length > MAX_POINTS) this.history.shift();
      this.prev = { s, at };
      this.emit();
    } catch (e) {
      this.error = e instanceof ApiError ? e.message : String(e);
      this.prev = null; // counters are only comparable within one connection
      this.emit();
      if (e instanceof ApiError && e.code === 'monitor_unavailable') {
        this.stopped = true; // not a Linux server: asking again is pointless
        return;
      }
    } finally {
      this.running = false;
    }
    this.schedule();
  }

  private toPoint(a: { s: Sample; at: number }, b: { s: Sample; at: number }): Point {
    const dt = Math.max(0.05, (b.at - a.at) / 1000);
    const delta = (x: [number, number] | null | undefined, y: [number, number] | null | undefined) => (x && y ? pct(y[0] - x[0], y[1] - x[1]) : null);
    const rate = (x: number | undefined, y: number | undefined) => (x === undefined || y === undefined || y < x ? 0 : (y - x) / dt);
    const m = b.s.mem;
    const ops = (c: number[], o: number[]) => c[0] - o[0] + (c[3] - o[3]);
    const ifaces: IfacePoint[] = (b.s.ifaces ?? []).map((i) => {
      const o = a.s.ifaces?.find((x) => x.name === i.name);
      return { name: i.name, rx: rate(o?.rx, i.rx), tx: rate(o?.tx, i.tx) };
    });
    const disks: DiskPoint[] = (b.s.disks ?? []).map((d) => {
      const o = a.s.disks?.find((x) => x.name === d.name)?.c;
      const c = d.c;
      if (!o || c.some((v, i) => i < 7 && v < o[i])) return { name: d.name, r: 0, w: 0, ri: 0, wi: 0, util: 0, queue: c[7], resp: 0 };
      return { name: d.name, r: ((c[1] - o[1]) * 512) / dt, w: ((c[4] - o[4]) * 512) / dt, ri: (c[0] - o[0]) / dt, wi: (c[3] - o[3]) / dt, util: Math.min(100, ((c[6] - o[6]) / (dt * 1000)) * 100), queue: c[7], resp: ops(c, o) > 0 ? (c[2] - o[2] + (c[5] - o[5])) / ops(c, o) : 0 };
    });
    return {
      cpu: delta(a.s.cpu, b.s.cpu),
      cores: b.s.cores.map((c, i) => delta(a.s.cores[i], c) ?? 0),
      mem: m && m.total > 0 ? ((m.total - m.available) / m.total) * 100 : null,
      swap: m && m.swap_total > 0 ? ((m.swap_total - m.swap_free) / m.swap_total) * 100 : null,
      load: b.s.load?.l1 ?? null,
      disks,
      ifaces,
      mhz: b.s.cpu_mhz ?? null,
      dr: disks.reduce((n, d) => n + d.r, 0),
      dw: disks.reduce((n, d) => n + d.w, 0),
      rx: rate(a.s.net?.rx, b.s.net?.rx),
      tx: rate(a.s.net?.tx, b.s.net?.tx),
    };
  }
}

// ------------------------------------------------------------------ drawing

export const COLORS = { cpu: '#60cdff', mem: '#b4a0ff', swap: '#ff9ac1', rx: '#6ccb5f', tx: '#ffb454', grid: 'rgba(255,255,255,.08)' };

interface Series {
  data: (number | null)[];
  color: string;
  fill?: boolean;
  /** Dashed line (writes / sent data, like in the Windows task manager). */
  dash?: boolean;
}

/** Line / area chart on a canvas (crisp at any DPI). `max` fixed (percent) or auto from the data. */
export function drawChart(canvas: HTMLCanvasElement, series: Series[], opts: { max?: number; grid?: boolean; min?: number; frame?: string; vgrid?: boolean; points?: number } = {}): void {
  const dpr = window.devicePixelRatio || 1;
  const w = canvas.clientWidth;
  const hgt = canvas.clientHeight;
  if (!w || !hgt) return;
  if (canvas.width !== Math.round(w * dpr) || canvas.height !== Math.round(hgt * dpr)) {
    canvas.width = Math.round(w * dpr);
    canvas.height = Math.round(hgt * dpr);
  }
  const ctx = canvas.getContext('2d');
  if (!ctx) return;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, hgt);
  const maxData = Math.max(0, ...series.flatMap((s) => s.data.map((v) => v ?? 0)));
  const max = opts.max ?? Math.max(opts.min ?? 1, maxData * 1.15);
  if (opts.grid) {
    ctx.strokeStyle = COLORS.grid;
    ctx.lineWidth = 1;
    if (opts.vgrid) {
      for (let i = 1; i < 10; i++) {
        const x = Math.round((w * i) / 10) + 0.5;
        ctx.beginPath();
        ctx.moveTo(x, 0);
        ctx.lineTo(x, hgt);
        ctx.stroke();
      }
    }
    for (const f of opts.vgrid ? [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9] : [0.25, 0.5, 0.75]) {
      const y = Math.round(hgt * (1 - f)) + 0.5;
      ctx.beginPath();
      ctx.moveTo(0, y);
      ctx.lineTo(w, y);
      ctx.stroke();
    }
  }
  // the x scale grows with the history (at least 30 points wide) so that a fresh graph is not squeezed to the edge
  const n = opts.points ?? Math.max(30, ...series.map((s) => s.data.length));
  for (const s of series) {
    const d = s.data;
    if (!d.length) continue;
    const step = w / (n - 1);
    const x0 = w - (d.length - 1) * step; // newest sample at the right edge
    const y = (v: number) => hgt - 1 - (Math.min(v, max) / max) * (hgt - 2);
    ctx.beginPath();
    let started = false;
    d.forEach((v, i) => {
      if (v === null) return;
      const px = x0 + i * step;
      if (!started) {
        ctx.moveTo(px, y(v));
        started = true;
      } else ctx.lineTo(px, y(v));
    });
    ctx.strokeStyle = s.color;
    ctx.lineWidth = 1.5;
    ctx.lineJoin = 'round';
    ctx.setLineDash(s.dash ? [4, 3] : []);
    ctx.stroke();
    ctx.setLineDash([]);
    if (s.fill && started) {
      ctx.lineTo(w, hgt);
      ctx.lineTo(x0, hgt);
      ctx.closePath();
      ctx.fillStyle = s.color + '33';
      ctx.fill();
    }
  }
  if (opts.frame) {
    ctx.strokeStyle = opts.frame;
    ctx.lineWidth = 1;
    ctx.strokeRect(0.5, 0.5, w - 1, hgt - 1);
  }
}

export const level = (v: number | null): string => (v === null ? '' : v >= 90 ? 'crit' : v >= 70 ? 'warn' : '');

// ------------------------------------------------------------------ the bar

export class MonitorBar {
  readonly el: HTMLElement;
  private nums = h('div', { class: 'mon-nums', 'data-testid': 'mon-nums' });
  private graphs = h('div', { class: 'mon-graphs' });
  private cCpu = h('canvas', { class: 'mon-spark', title: 'Processor load' }) as HTMLCanvasElement;
  private cMem = h('canvas', { class: 'mon-spark', title: 'Memory' }) as HTMLCanvasElement;
  private cNet = h('canvas', { class: 'mon-spark', title: 'Network' }) as HTMLCanvasElement;
  private off: () => void;
  private raf = 0;

  constructor(
    private src: MonitorSource,
    onOpen: () => void,
    private unavailable?: string,
  ) {
    this.graphs.append(this.cCpu, this.cMem, this.cNet);
    this.el = h('div', { class: 'mon-bar', role: 'button', tabindex: '0', title: 'Click to open the detailed monitor', 'data-testid': 'monitor-bar', onclick: onOpen, onkeydown: (e) => ((e as KeyboardEvent).key === 'Enter' && onOpen()) }, this.nums, h('div', { class: 'mon-gap' }), this.graphs);
    this.off = src.subscribe(() => this.schedule());
    this.render();
    new ResizeObserver(() => this.schedule()).observe(this.el);
  }

  dispose(): void {
    this.off();
  }

  setUnavailable(msg?: string): void {
    this.unavailable = msg;
    this.schedule();
  }

  setVisible(v: boolean): void {
    this.el.style.display = v ? '' : 'none';
  }

  private schedule(): void {
    if (this.raf) return;
    this.raf = requestAnimationFrame(() => {
      this.raf = 0;
      this.render();
    });
  }

  private item(label: string, value: string, cls = '', testid?: string): HTMLElement {
    return h('span', { class: 'mon-item ' + cls, 'data-testid': testid }, h('span', { class: 'mon-l' }, label), h('span', { class: 'mon-v' }, value));
  }

  private render(): void {
    clear(this.nums);
    const s = this.src.last;
    const p = this.src.history[this.src.history.length - 1];
    if (this.unavailable || (!s && this.src.error)) {
      this.nums.append(h('span', { class: 'mon-off', 'data-testid': 'mon-off' }, this.unavailable ?? this.src.error ?? ''));
      this.graphs.style.display = 'none';
      return;
    }
    this.graphs.style.display = '';
    if (!s) {
      this.nums.append(h('span', { class: 'mon-off' }, 'Collecting data…'));
      return;
    }
    const mem = s.mem;
    const used = mem ? mem.total - mem.available : 0;
    const memPct = p?.mem ?? (mem && mem.total ? (used / mem.total) * 100 : null);
    this.nums.append(
      this.item('CPU', p?.cpu == null ? '…' : `${Math.round(p.cpu)}%`, level(p?.cpu ?? null), 'mon-cpu'),
      this.item('RAM', memPct === null || !mem ? '—' : `${Math.round(memPct)}% · ${fmtBytes(used)} of ${fmtBytes(mem.total)}`, level(memPct), 'mon-mem'),
      ...(mem && mem.swap_total > 0 ? [this.item('Swap', `${Math.round(p?.swap ?? ((mem.swap_total - mem.swap_free) / mem.swap_total) * 100)}%`, level(p?.swap ?? null))] : []),
      ...(s.load ? [this.item('Load', `${s.load.l1.toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`, '', 'mon-load')] : []),
      this.item('Network', p ? `↓ ${fmtRate(p.rx)}  ↑ ${fmtRate(p.tx)}` : '…', '', 'mon-net'),
      this.item('Uptime', fmtUptime(s.uptime)),
    );
    const hist = this.src.history;
    drawChart(this.cCpu, [{ data: hist.map((x) => x.cpu), color: COLORS.cpu, fill: true }], { max: 100 });
    drawChart(this.cMem, [{ data: hist.map((x) => x.mem), color: COLORS.mem, fill: true }], { max: 100 });
    drawChart(this.cNet, [{ data: hist.map((x) => x.rx), color: COLORS.rx }, { data: hist.map((x) => x.tx), color: COLORS.tx }], { min: 10 * 1024 });
  }
}

