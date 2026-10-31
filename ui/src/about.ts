// "About": version, author, and the offline list of open-source licences (generated, embedded in the exe).
import { h, modal } from './dom';

declare const __APP_VERSION__: string;

async function licensesDialog(): Promise<void> {
  const text = (await import('./licenses.txt?raw')).default;
  await modal<boolean>((close) =>
    h(
      'div',
      { role: 'dialog', class: 'wide', 'data-testid': 'licenses-dialog' },
      h('h3', {}, 'Open-source library licences'),
      h('pre', { class: 'detail lic', style: 'display:block', 'data-testid': 'licenses-text' }, text),
      h('div', { class: 'row end' }, h('button', { class: 'primary', onclick: () => close(true), 'data-testid': 'licenses-close' }, 'Close')),
    ),
  );
}

export async function aboutDialog(): Promise<void> {
  await modal<boolean>((close) =>
    h(
      'div',
      { role: 'dialog', 'data-testid': 'about-dialog' },
      h('h3', {}, 'd256 Shell'),
      h('p', {}, 'Portable SSH client for Windows and Linux: terminal, SFTP file explorer and built-in editor. Data is stored in the data folder next to the program (in Linux, when installed from a package, in ~/.local/share/d256shell).'),
      h('div', { class: 'prop-row' }, h('span', { class: 'muted' }, 'Version'), h('span', { 'data-testid': 'about-version' }, __APP_VERSION__)),
      h('div', { class: 'prop-row' }, h('span', { class: 'muted' }, 'Author'), h('span', { 'data-testid': 'about-author' }, 'dest1256')),
      h('div', { class: 'prop-row' }, h('span', { class: 'muted' }, 'License'), h('span', {}, '© 2026 dest1256. MIT License: free to use, copy and modify as long as the copyright notice is kept.')),
      h('div', { class: 'row end' }, h('button', { onclick: () => void licensesDialog(), 'data-testid': 'open-licenses' }, 'Library licences'), h('button', { class: 'primary', onclick: () => close(true), 'data-testid': 'about-close' }, 'Close')),
    ),
  );
}
