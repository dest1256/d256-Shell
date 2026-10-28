// The "Монитор" tab: a resource list on the left and the details of the selected resource on the right, laid out
// like the "Performance" tab of the Windows 10 task manager (CPU, memory, every disk, every network interface).
import { clear, h } from './dom';
import { getSettings, updateSettings } from './settings';
import { drawChart, fmtBits, fmtBytes, fmtDuration, fmtRate, MonitorSource, Point, Sample } from './monitor';

/** Resource colours, as in the task manager: blue CPU, purple memory, green disks, orange network. */
const C = { cpu: '#4aa8e8', mem: '#b072e8', disk: '#5cc258', net: '#f08a3c' };

type Key = 'cpu' | 'mem' | `disk:${string}` | `net:${string}`;

const fmt1 = (v: number) => v.toLocaleString('ru-RU', { minimumFractionDigits: 1, maximumFractionDigits: 1 });
const fmtPct = (v: number | null) => (v === null ? '…' : `${Math.round(v)}%`);
const fmtGhz = (mhz: number | null | undefined) => (mhz ? `${(mhz / 1000).toLocaleString('ru-RU', { minimumFractionDigits: 2, maximumFractionDigits: 2 })} ГГц` : '—');

/** The part of the history shown on the graphs (about a minute, like the task manager). */
function windowOf(hist: Point[], intervalSec: number): { pts: Point[]; n: number } {
  const n = Math.max(20, Math.round(60 / Math.max(0.1, intervalSec)) + 1);
  return { pts: hist.slice(-n), n };
}

interface View {
  el: HTMLElement;
  update(s: Sample, hist: Point[], intervalSec: number, info: Sample['info']): void;
}

function stat(id: string, label: string, color?: string): { el: HTMLElement; set: (v: string) => void } {
  const v = h('div', { class: 'tm-sv', 'data-testid': `stat-${id}` }, '—');
  const el = h('div', { class: 'tm-stat' + (color ? ' bar' : '') }, h('div', { class: 'tm-sl' }, label), v);
  if (color) el.style.borderLeftColor = color;
  return { el, set: (t) => (v.textContent = t) };
}

function table(rows: [string, string][]): { el: HTMLElement; set: (i: number, v: string) => void } {
  const vals = rows.map(([, v]) => h('span', { class: 'tm-tv' }, v));
  const el = h('div', { class: 'tm-table' });
  rows.forEach(([k], i) => el.append(h('span', { class: 'tm-tk' }, `${k}:`), vals[i]));
  return { el, set: (i, v) => (vals[i].textContent = v) };
}

function chartBox(id: string, caption: string, color: string, height: number): { el: HTMLElement; canvas: HTMLCanvasElement; peak: HTMLElement } {
  const canvas = h('canvas', { class: 'tm-chart', style: `height:${height}px;border-color:${color}`, 'data-testid': `md-chart-${id}` }) as HTMLCanvasElement;
  const peak = h('span', { class: 'tm-peak' });
  return { el: h('div', { class: 'tm-cbox' }, h('div', { class: 'tm-cap' }, h('span', {}, caption), peak), canvas), canvas, peak };
}

function head(title: string, right: string): { el: HTMLElement; setRight: (t: string) => void } {
  const r = h('div', { class: 'tm-model' }, right);
  return { el: h('div', { class: 'tm-head' }, h('h2', { 'data-testid': 'md-title' }, title), r), setRight: (t) => (r.textContent = t) };
}

// ------------------------------------------------------------------ CPU

function cpuView(onToggle: () => void, total: boolean): View {
  const hd = head('ЦП', '');
  const cap = h('div', { class: 'tm-cap' });
  const toggle = h('button', { class: 'tool-sm' + (total ? ' on' : ''), 'data-testid': 'md-cpu-total', title: 'Одна общая диаграмма вместо отдельной на каждое ядро', onclick: onToggle }, 'Общая загрузка');
  const grid = h('div', { class: 'tm-cores', 'data-testid': 'md-cores' });
  const one = chartBox('cpu', '', C.cpu, 280);
  const util = stat('cpu-util', 'Использование');
  const speed = stat('cpu-speed', 'Скорость');
  const threads = stat('cpu-threads', 'Потоки (задачи)');
  const up = stat('cpu-uptime', 'Время работы');
  const tb = table([
    ['Логических процессоров', '—'],
    ['Нагрузка за 1 / 5 / 15 мин', '—'],
    ['Выполняется сейчас', '—'],
  ]);
  const canv: HTMLCanvasElement[] = [];
  const el = h(
    'div',
    { class: 'tm-pane' },
    hd.el,
    h('div', { class: 'tm-row' }, cap, toggle),
    total ? one.el : grid,
    h('div', { class: 'tm-stats' }, h('div', { class: 'tm-sgrid' }, util.el, speed.el, threads.el, up.el), tb.el),
  );
  return {
    el,
    update(s, hist, iv, info) {
      const { pts, n } = windowOf(hist, iv);
      const last = pts[pts.length - 1];
      hd.setRight(info?.cpu_model ?? '');
      cap.textContent = `% использования за ${Math.round((n - 1) * iv)} с`;
      one.peak.textContent = '100%';
      util.set(fmtPct(last?.cpu ?? null));
      speed.set(fmtGhz(s.cpu_mhz));
      threads.set(s.load ? String(s.load.procs) : '—');
      up.set(fmtDuration(s.uptime));
      tb.set(0, String(s.cores.length || '—'));
      tb.set(1, s.load ? [s.load.l1, s.load.l5, s.load.l15].map((x) => x.toLocaleString('ru-RU', { minimumFractionDigits: 2, maximumFractionDigits: 2 })).join(' / ') : '—');
      tb.set(2, s.load ? String(s.load.running) : '—');
      if (total || s.cores.length <= 1) {
        drawChart(one.canvas, [{ data: pts.map((p) => p.cpu), color: C.cpu, fill: true }], { max: 100, grid: true, vgrid: true, frame: C.cpu, points: n });
        return;
      }
      // like the task manager: all cores fit on the screen (more columns for more cores)
      const cols = s.cores.length <= 4 ? 2 : s.cores.length <= 9 ? 3 : s.cores.length <= 16 ? 4 : 8;
      grid.style.gridTemplateColumns = `repeat(${cols}, 1fr)`;
      while (canv.length < s.cores.length) {
        const c = h('canvas', { class: 'tm-core', style: `border-color:${C.cpu}`, title: `Логический процессор ${canv.length}` }) as HTMLCanvasElement;
        canv.push(c);
        grid.append(c);
      }
      canv.forEach((c, i) => drawChart(c, [{ data: pts.map((p) => p.cores[i] ?? null), color: C.cpu, fill: true }], { max: 100, grid: true, vgrid: true, frame: C.cpu, points: n }));
    },
  };
}

// ------------------------------------------------------------------ memory

function memView(): View {
  const hd = head('Память', '');
  const cu = chartBox('mem', 'Использование памяти', C.mem, 240);
  const bar = h('div', { class: 'tm-membar', 'data-testid': 'md-membar' });
  const segs = [h('div', { class: 'seg used', title: 'Используется' }), h('div', { class: 'seg cache', title: 'Кэш и буферы' }), h('div', { class: 'seg free', title: 'Свободно' })];
  bar.append(...segs);
  const used = stat('mem-used', 'Используется');
  const avail = stat('mem-avail', 'Доступно');
  const cached = stat('mem-cached', 'Кэшировано');
  const free = stat('mem-free', 'Свободно');
  const tb = table([
    ['Всего', '—'],
    ['Своп занят', '—'],
    ['Своп всего', '—'],
  ]);
  const el = h(
    'div',
    { class: 'tm-pane' },
    hd.el,
    cu.el,
    h('div', { class: 'tm-cap' }, h('span', {}, 'Структура памяти: используется, кэш, свободно')),
    bar,
    h('div', { class: 'tm-stats' }, h('div', { class: 'tm-sgrid' }, used.el, avail.el, cached.el, free.el), tb.el),
  );
  return {
    el,
    update(s, hist, iv, info) {
      const { pts, n } = windowOf(hist, iv);
      const m = s.mem;
      if (!m) return;
      const usedB = m.total - m.available;
      const cacheB = Math.max(0, m.available - m.free);
      hd.setRight(fmtBytes(m.total));
      cu.peak.textContent = fmtBytes(m.total);
      drawChart(cu.canvas, [{ data: pts.map((p) => p.mem), color: C.mem, fill: true }, { data: pts.map((p) => p.swap), color: '#ff9ac1', dash: true }], { max: 100, grid: true, vgrid: true, frame: C.mem, points: n });
      segs[0].style.flexGrow = String(usedB);
      segs[1].style.flexGrow = String(cacheB);
      segs[2].style.flexGrow = String(m.free);
      used.set(`${fmtBytes(usedB)} (${fmtPct((usedB / m.total) * 100)})`);
      avail.set(fmtBytes(m.available));
      cached.set(fmtBytes(m.cached));
      free.set(fmtBytes(m.free));
      tb.set(0, fmtBytes(m.total));
      tb.set(1, m.swap_total ? fmtBytes(m.swap_total - m.swap_free) : 'нет');
      tb.set(2, m.swap_total ? fmtBytes(m.swap_total) : 'нет');
    },
  };
}

// ------------------------------------------------------------------ disk

function diskView(name: string): View {
  const hd = head(`Диск ${name}`, '');
  const act = chartBox(`disk-${name}-active`, 'Активное время', C.disk, 200);
  const io = chartBox(`disk-${name}-io`, 'Скорость обмена данными с диском (сплошная — чтение, пунктир — запись)', C.disk, 110);
  const active = stat('disk-active', 'Активное время', C.disk);
  const resp = stat('disk-resp', 'Среднее время ответа');
  const rd = stat('disk-read', 'Скорость чтения', C.disk);
  const wr = stat('disk-write', 'Скорость записи', C.disk);
  wr.el.classList.add('dashed');
  const tb = table([
    ['Ёмкость', '—'],
    ['Тип', '—'],
    ['Модель', '—'],
    ['Операций в секунду (чт / зап)', '—'],
    ['Очередь запросов', '—'],
  ]);
  const el = h('div', { class: 'tm-pane' }, hd.el, act.el, io.el, h('div', { class: 'tm-stats' }, h('div', { class: 'tm-sgrid' }, active.el, resp.el, rd.el, wr.el), tb.el));
  return {
    el,
    update(s, hist, iv, info) {
      const { pts, n } = windowOf(hist, iv);
      const dev = info?.devices?.disks?.[name];
      const d = (p: Point | undefined) => p?.disks.find((x) => x.name === name);
      const last = d(pts[pts.length - 1]);
      hd.setRight(dev?.model ?? '');
      act.peak.textContent = '100%';
      const series = (f: (x: NonNullable<ReturnType<typeof d>>) => number) => pts.map((p) => (d(p) ? f(d(p)!) : null));
      const peak = Math.max(0, ...series((x) => x.r).map((v) => v ?? 0), ...series((x) => x.w).map((v) => v ?? 0));
      io.peak.textContent = fmtRate(Math.max(peak * 1.15, 100 * 1024)); // the same scale as the chart
      drawChart(act.canvas, [{ data: series((x) => x.util), color: C.disk, fill: true }], { max: 100, grid: true, vgrid: true, frame: C.disk, points: n });
      drawChart(io.canvas, [{ data: series((x) => x.r), color: C.disk, fill: true }, { data: series((x) => x.w), color: C.disk, dash: true }], { min: 100 * 1024, grid: true, vgrid: true, frame: C.disk, points: n });
      active.set(fmtPct(last?.util ?? null));
      resp.set(last ? `${fmt1(last.resp)} мс` : '…');
      rd.set(last ? fmtRate(last.r) : '…');
      wr.set(last ? fmtRate(last.w) : '…');
      tb.set(0, dev?.size ? fmtBytes(dev.size) : '—');
      tb.set(1, dev?.kind || '—');
      tb.set(2, dev?.model || '—');
      tb.set(3, last ? `${Math.round(last.ri)} / ${Math.round(last.wi)}` : '…');
      tb.set(4, last ? String(last.queue) : '…');
    },
  };
}

// ------------------------------------------------------------------ network

function netView(name: string): View {
  const hd = head(name, '');
  const ch = chartBox(`net-${name}`, 'Пропускная способность (сплошная — приём, пунктир — отправка)', C.net, 300);
  const send = stat('net-send', 'Отправка', C.net);
  send.el.classList.add('dashed');
  const recv = stat('net-recv', 'Получение', C.net);
  const tb = table([
    ['Адаптер', name],
    ['Состояние', '—'],
    ['Скорость соединения', '—'],
    ['MAC-адрес', '—'],
    ['Принято всего', '—'],
    ['Отправлено всего', '—'],
  ]);
  const el = h('div', { class: 'tm-pane' }, hd.el, ch.el, h('div', { class: 'tm-stats' }, h('div', { class: 'tm-sgrid' }, send.el, recv.el), tb.el));
  return {
    el,
    update(s, hist, iv, info) {
      const { pts, n } = windowOf(hist, iv);
      const i = (p: Point | undefined) => p?.ifaces.find((x) => x.name === name);
      const last = i(pts[pts.length - 1]);
      const dev = info?.devices?.ifaces?.[name];
      const cur = s.ifaces?.find((x) => x.name === name);
      hd.setRight(dev?.speed_mbit ? `${dev.speed_mbit >= 1000 ? dev.speed_mbit / 1000 : dev.speed_mbit} ${dev.speed_mbit >= 1000 ? 'Гбит/с' : 'Мбит/с'}` : '');
      const rx = pts.map((p) => i(p)?.rx ?? null);
      const tx = pts.map((p) => i(p)?.tx ?? null);
      ch.peak.textContent = fmtBits(Math.max(125 * 1024, 1.15 * Math.max(0, ...rx.map((v) => v ?? 0), ...tx.map((v) => v ?? 0))));
      drawChart(ch.canvas, [{ data: rx, color: C.net, fill: true }, { data: tx, color: C.net, dash: true }], { min: 125 * 1024, grid: true, vgrid: true, frame: C.net, points: n });
      send.set(last ? fmtBits(last.tx) : '…');
      recv.set(last ? fmtBits(last.rx) : '…');
      tb.set(1, dev?.state ?? '—');
      tb.set(2, dev?.speed_mbit ? `${dev.speed_mbit} Мбит/с` : 'неизвестна');
      tb.set(3, dev?.mac ?? '—');
      tb.set(4, cur ? fmtBytes(cur.rx) : '—');
      tb.set(5, cur ? fmtBytes(cur.tx) : '—');
    },
  };
}

// ------------------------------------------------------------------ the tab

export class MonitorDetail {
  readonly el: HTMLElement;
  private side = h('div', { class: 'tm-side', role: 'listbox', 'data-testid': 'md-side' });
  private main = h('div', { class: 'tm-main', 'data-testid': 'md-main' });
  private selected: Key = 'cpu';
  private view: { key: string; v: View } | null = null;
  private cpuTotal = false;
  private off: () => void;
  private raf = 0;
  private thumbs = new Map<Key, HTMLCanvasElement>();

  constructor(private src: MonitorSource) {
    this.el = h('div', { class: 'monitor tm', 'data-testid': 'monitor' }, h('div', { class: 'tm-left' }, this.side, this.rateBox()), this.main);
    this.off = src.subscribe(() => this.schedule());
    new ResizeObserver(() => this.schedule()).observe(this.el);
    this.render();
  }

  /** "Update speed" like the task manager: pause, a preset (from 250 ms) or your own value in ms (from 100). */
  private rateBox(): HTMLElement {
    const PRESETS: [string, string][] = [['250', '250 мс'], ['500', '500 мс'], ['1000', '1 секунда'], ['2000', '2 секунды'], ['5000', '5 секунд'], ['10000', '10 секунд'], ['30000', '30 секунд']];
    const sel = h('select', { 'data-testid': 'md-rate', title: 'Как часто обновлять графики' }) as HTMLSelectElement;
    sel.append(h('option', { value: 'pause' }, 'Пауза'));
    for (const [v, t] of PRESETS) sel.append(h('option', { value: v }, `Каждые: ${t}`));
    sel.append(h('option', { value: 'custom' }, 'Своя частота…'));
    const input = h('input', { type: 'number', min: '100', max: '60000', step: '50', 'data-testid': 'md-rate-ms', title: 'Интервал в миллисекундах, от 100 до 60000' }) as HTMLInputElement;
    const unit = h('span', { class: 'muted' }, 'мс');
    const custom = h('div', { class: 'tm-rate-custom' }, input, unit);
    const err = h('div', { class: 'tm-rate-err', 'data-testid': 'md-rate-err' });
    const showCustom = (on: boolean) => (custom.style.display = on ? '' : 'none');
    const apply = async (ms: number) => {
      this.src.paused = false;
      await updateSettings({ monitor_interval_ms: ms });
      this.src.restart();
    };
    const sync = () => {
      const ms = getSettings().monitor_interval_ms;
      if (this.src.paused) sel.value = 'pause';
      else if (PRESETS.some(([v]) => Number(v) === ms)) sel.value = String(ms);
      else sel.value = 'custom';
      input.value = String(ms);
      showCustom(sel.value === 'custom');
    };
    sync();
    sel.addEventListener('change', () => {
      err.textContent = '';
      showCustom(sel.value === 'custom');
      if (sel.value === 'pause') {
        this.src.paused = true;
        this.src.restart();
      } else if (sel.value === 'custom') input.focus();
      else void apply(Number(sel.value));
    });
    const commit = () => {
      const ms = Math.round(Number(input.value));
      if (!Number.isFinite(ms) || ms < 100 || ms > 60000) {
        err.textContent = 'От 100 до 60000 мс';
        return;
      }
      err.textContent = '';
      void apply(ms);
    };
    input.addEventListener('change', commit);
    input.addEventListener('keydown', (e) => e.key === 'Enter' && commit());
    return h('div', { class: 'tm-rate' }, h('label', {}, 'Частота обновления', sel), custom, err);
  }

  dispose(): void {
    this.off();
  }

  private schedule(): void {
    if (this.raf) return;
    this.raf = requestAnimationFrame(() => {
      this.raf = 0;
      this.render();
    });
  }

  private items(s: Sample, hist: Point[], iv: number, info: Sample['info']): { key: Key; title: string; sub: string; color: string; data: (number | null)[]; max?: number; data2?: (number | null)[] }[] {
    const { pts } = windowOf(hist, iv);
    const last = pts[pts.length - 1];
    const m = s.mem;
    const out: ReturnType<MonitorDetail['items']> = [
      { key: 'cpu', title: 'ЦП', sub: `${fmtPct(last?.cpu ?? null)} ${fmtGhz(s.cpu_mhz)}`.trim(), color: C.cpu, data: pts.map((p) => p.cpu), max: 100 },
      { key: 'mem', title: 'Память', sub: m ? `${fmt1((m.total - m.available) / 1073741824)}/${fmt1(m.total / 1073741824)} ГБ (${fmtPct(last?.mem ?? null)})` : '', color: C.mem, data: pts.map((p) => p.mem), max: 100 },
    ];
    (s.disks ?? []).forEach((d, idx) => {
      const dev = info?.devices?.disks?.[d.name];
      const dp = last?.disks.find((x) => x.name === d.name);
      out.push({ key: `disk:${d.name}`, title: `Диск ${idx} (${d.name})`, sub: `${dev?.kind ? dev.kind + ' ' : ''}${fmtPct(dp?.util ?? null)}`, color: C.disk, data: pts.map((p) => p.disks.find((x) => x.name === d.name)?.util ?? null), max: 100 });
    });
    (s.ifaces ?? []).forEach((i) => {
      const ip = last?.ifaces.find((x) => x.name === i.name);
      out.push({ key: `net:${i.name}`, title: i.name, sub: ip ? `О: ${fmtBits(ip.tx)} П: ${fmtBits(ip.rx)}` : '…', color: C.net, data: pts.map((p) => p.ifaces.find((x) => x.name === i.name)?.rx ?? null), data2: pts.map((p) => p.ifaces.find((x) => x.name === i.name)?.tx ?? null) });
    });
    return out;
  }

  private viewFor(key: Key): View {
    if (key === 'cpu') return cpuView(() => ((this.cpuTotal = !this.cpuTotal), (this.view = null), this.schedule()), this.cpuTotal);
    if (key === 'mem') return memView();
    if (key.startsWith('disk:')) return diskView(key.slice(5));
    return netView(key.slice(4));
  }

  private render(): void {
    const s = this.src.last;
    if (!s) {
      clear(this.side);
      clear(this.main);
      this.view = null;
      this.main.append(h('div', { class: 'muted tm-wait', 'data-testid': 'md-wait' }, this.src.error ?? 'Сбор данных… (нужен второй замер, это займёт пару секунд)'));
      return;
    }
    const iv = this.src.intervalSec;
    const items = this.items(s, this.src.history, iv, this.src.info);
    if (!items.some((x) => x.key === this.selected)) this.selected = 'cpu';

    // left: the list of resources, each with a small live graph
    clear(this.side);
    this.thumbs.clear();
    const { n } = windowOf(this.src.history, iv);
    for (const it of items) {
      const th = h('canvas', { class: 'tm-thumb', style: `border-color:${it.color}` }) as HTMLCanvasElement;
      this.thumbs.set(it.key, th);
      this.side.append(
        h(
          'button',
          { class: 'tm-item' + (it.key === this.selected ? ' active' : ''), role: 'option', 'aria-selected': String(it.key === this.selected), 'data-testid': `md-item-${it.key.replace(':', '-')}`, onclick: () => ((this.selected = it.key), this.schedule()) },
          th,
          h('div', { class: 'tm-it' }, h('div', { class: 'tm-name' }, it.title), h('div', { class: 'tm-sub' }, it.sub)),
        ),
      );
      drawChart(th, [{ data: it.data, color: it.color, fill: true }, ...(it.data2 ? [{ data: it.data2, color: it.color, dash: true }] : [])], { max: it.max, min: it.max ? undefined : 125 * 1024, frame: it.color, points: n });
    }

    // right: details of the selected resource (built once per selection, then only updated)
    if (!this.view || this.view.key !== this.selected) {
      this.view = { key: this.selected, v: this.viewFor(this.selected) };
      clear(this.main);
      this.main.append(this.view.v.el);
    }
    this.view.v.update(s, this.src.history, iv, this.src.info);
  }
}
