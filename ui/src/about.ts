// "О программе": version, author, and the offline list of open-source licences (generated, embedded in the exe).
import { h, modal } from './dom';

declare const __APP_VERSION__: string;

async function licensesDialog(): Promise<void> {
  const text = (await import('./licenses.txt?raw')).default;
  await modal<boolean>((close) =>
    h(
      'div',
      { role: 'dialog', class: 'wide', 'data-testid': 'licenses-dialog' },
      h('h3', {}, 'Лицензии открытых библиотек'),
      h('pre', { class: 'detail lic', style: 'display:block', 'data-testid': 'licenses-text' }, text),
      h('div', { class: 'row end' }, h('button', { class: 'primary', onclick: () => close(true), 'data-testid': 'licenses-close' }, 'Закрыть')),
    ),
  );
}

export async function aboutDialog(): Promise<void> {
  await modal<boolean>((close) =>
    h(
      'div',
      { role: 'dialog', 'data-testid': 'about-dialog' },
      h('h3', {}, 'd256 Shell'),
      h('p', {}, 'Портативный SSH-клиент для Windows и Linux: терминал, файловый проводник по SFTP и встроенный редактор. Данные хранятся в папке data рядом с программой (в Linux при установке из пакета — в ~/.local/share/d256shell).'),
      h('div', { class: 'prop-row' }, h('span', { class: 'muted' }, 'Версия'), h('span', { 'data-testid': 'about-version' }, __APP_VERSION__)),
      h('div', { class: 'prop-row' }, h('span', { class: 'muted' }, 'Автор'), h('span', { 'data-testid': 'about-author' }, 'dest1256')),
      h('div', { class: 'prop-row' }, h('span', { class: 'muted' }, 'Права'), h('span', {}, '© 2026 dest1256. Все права защищены. Приватное программное обеспечение.')),
      h('div', { class: 'row end' }, h('button', { onclick: () => void licensesDialog(), 'data-testid': 'open-licenses' }, 'Лицензии библиотек'), h('button', { class: 'primary', onclick: () => close(true), 'data-testid': 'about-close' }, 'Закрыть')),
    ),
  );
}
