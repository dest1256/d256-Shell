// Thin layer over the Tauri-only features: native pickers, drop from Explorer, native drag-out.
// In a plain browser (e2e through devbridge) there is no native UI: the picker degrades to a text prompt,
// and the others to no-ops.
import { invoke } from '@tauri-apps/api/core';
import { inTauri, Native } from './api';
import { promptDialog } from './dom';
import { t } from './i18n';

async function pick(kind: 'files' | 'folder'): Promise<string[]> {
  if (inTauri) {
    const { open } = await import('@tauri-apps/plugin-dialog');
    const r = await open({ multiple: kind === 'files', directory: kind === 'folder' });
    if (!r) return [];
    return Array.isArray(r) ? r : [r];
  }
  const txt = await promptDialog(kind === 'files' ? t('Upload files') : t('Choose a folder'), kind === 'files' ? t('Local paths (separated by |)') : t('Local folder'), '', 'OK');
  return txt ? txt.split(/\r?\n|\|/).map((s) => s.trim()).filter(Boolean) : [];
}

export const native: Native = {
  isTauri: inTauri,
  pick,
  async dragOut(paths) {
    if (!inTauri) throw new Error(t('Dragging out only works in the app.'));
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
