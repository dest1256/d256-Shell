// Thin layer over the Tauri-only features: native pickers, drop from Explorer, native drag-out.
// In a plain browser (e2e through devbridge) there is no native UI: the picker degrades to a text prompt,
// and the others to no-ops.
import { invoke } from '@tauri-apps/api/core';
import { inTauri, Native } from './api';
import { promptDialog } from './dom';

async function pick(kind: 'files' | 'folder'): Promise<string[]> {
  if (inTauri) {
    const { open } = await import('@tauri-apps/plugin-dialog');
    const r = await open({ multiple: kind === 'files', directory: kind === 'folder' });
    if (!r) return [];
    return Array.isArray(r) ? r : [r];
  }
  const t = await promptDialog(kind === 'files' ? 'Загрузить файлы' : 'Выберите папку', kind === 'files' ? 'Локальные пути (через |)' : 'Локальная папка', '', 'OK');
  return t ? t.split(/\r?\n|\|/).map((s) => s.trim()).filter(Boolean) : [];
}

export const native: Native = {
  isTauri: inTauri,
  pick,
  async dragOut(paths) {
    if (!inTauri) throw new Error('Перетаскивание наружу работает только в приложении.');
    await invoke('drag_out', { paths });
  },
  async onFileDrop(h) {
    if (!inTauri) return () => undefined;
    const { getCurrentWebview } = await import('@tauri-apps/api/webview');
    // Tauri reports physical pixels: convert to CSS pixels so that elementFromPoint works at any DPI scale.
    const css = (p: { x: number; y: number }) => ({ x: p.x / window.devicePixelRatio, y: p.y / window.devicePixelRatio });
    return getCurrentWebview().onDragDropEvent((ev) => {
      const p = ev.payload;
      if (p.type === 'enter') h.enter(css(p.position).x, css(p.position).y);
      else if (p.type === 'over') h.over(css(p.position).x, css(p.position).y);
      else if (p.type === 'drop') h.drop(p.paths, css(p.position).x, css(p.position).y);
      else h.leave();
    });
  },
};
