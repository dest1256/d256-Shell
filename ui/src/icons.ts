// One lightweight embedded SVG icon set (24x24, stroke). No external assets.
const P: Record<string, string> = {
  plus: 'M12 5v14M5 12h14',
  server: 'M4 4h16v6H4zM4 14h16v6H4zM8 7h.01M8 17h.01',
  terminal: 'M4 5h16v14H4zM7 9l3 3-3 3M12 15h5',
  lock: 'M6 11h12v9H6zM8 11V8a4 4 0 0 1 8 0v3',
  unlock: 'M6 11h12v9H6zM8 11V8a4 4 0 0 1 7-2.6',
  key: 'M15 8a4 4 0 1 1-3.9 3L4 18v2h3v-2h2v-2h2l1.2-1.2A4 4 0 0 1 15 8zM16 8h.01',
  edit: 'M4 20h4L19 9l-4-4L4 16zM13 7l4 4',
  copy: 'M9 9h11v11H9zM5 15V4h11',
  trash: 'M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3',
  folder: 'M3 6h6l2 2h10v11H3z',
  search: 'M11 4a7 7 0 1 0 0 14 7 7 0 0 0 0-14zM21 21l-5-5',
  home: 'M4 11l8-7 8 7v9h-5v-6H9v6H4z',
  sidebar: 'M4 5h16v14H4zM9 5v14',
  settings: 'M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6zM19 12l2-1-1-3-2 .3-1.5-1.5.3-2-3-1-1 2h-2l-1-2-3 1 .3 2L6 7.8 4 7.5l-1 3 2 1v2l-2 1 1 3 2-.3 1.5 1.5-.3 2 3 1 1-2h2l1 2 3-1-.3-2 1.5-1.5 2 .3 1-3-2-1z',
  min: 'M5 12h14',
  max: 'M6 6h12v12H6z',
  restore: 'M8 8h10v10H8zM6 15V6h9',
  close: 'M6 6l12 12M18 6L6 18',
  check: 'M5 12l5 5 9-10',
  up: 'M12 19V5M5 12l7-7 7 7',
  down: 'M12 5v14M5 12l7 7 7-7',
  refresh: 'M20 11a8 8 0 1 0-2.3 5.7M20 4v7h-7',
  link: 'M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1',
  file: 'M6 3h8l4 4v14H6zM14 3v4h4',
  'folder-plus': 'M3 6h6l2 2h10v11H3zM12 11v6M9 14h6',
  'file-plus': 'M6 3h8l4 4v14H6zM14 3v4h4M12 11v6M9 14h6',
  cut: 'M6 6a2 2 0 1 0 0 .01M6 18a2 2 0 1 0 0 .01M8 7l12 11M8 17L20 6',
  paste: 'M9 4h6v3H9zM7 5H5v16h14V5h-2M9 12h6M9 16h6',
  rename: 'M4 20h16M6 16l1-4 8-8 3 3-8 8z',
  upload: 'M12 16V4M6 10l6-6 6 6M4 20h16',
  download: 'M12 4v12M6 10l6 6 6-6M4 20h16',
  info: 'M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM12 8h.01M11 12h1v5',
  activity: 'M3 12h4l3-8 4 16 3-8h4',
  plug: 'M9 3v5M15 3v5M6 8h12v3a6 6 0 0 1-12 0zM12 17v4',
};

export function icon(name: keyof typeof P | string, size = 16): SVGElement {
  const ns = 'http://www.w3.org/2000/svg';
  const svg = document.createElementNS(ns, 'svg');
  svg.setAttribute('viewBox', '0 0 24 24');
  svg.setAttribute('width', String(size));
  svg.setAttribute('height', String(size));
  svg.setAttribute('fill', 'none');
  svg.setAttribute('stroke', 'currentColor');
  svg.setAttribute('stroke-width', '1.8');
  svg.setAttribute('stroke-linecap', 'round');
  svg.setAttribute('stroke-linejoin', 'round');
  svg.setAttribute('aria-hidden', 'true');
  const path = document.createElementNS(ns, 'path');
  path.setAttribute('d', P[name] ?? '');
  svg.append(path);
  return svg;
}
