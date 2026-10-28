// Settings store: mirrors settings.json (owned by the backend), notifies subscribers on change.
import { api, Settings } from './api';

const DEFAULTS: Settings = {
  ssh_path: '',
  auto_password: 'auto',
  font_family: "'Cascadia Mono', Consolas, 'Courier New', monospace",
  font_size: 14,
  scheme: 'campbell',
  cursor_style: 'bar',
  scrollback: 10000,
  right_click: 'copy-paste',
  wheel_mode: 'arrows',
  parallel_transfers: 2,
  keep_mtime: true,
  drag_out_limit_mb: 200,
  editor_limit_mb: 5,
  auto_lock_minutes: 0,
  clone_mode: 'ask',
  monitor_enabled: true,
  monitor_interval_ms: 2000,
  paste_warn_lines: 2,
  paste_warn_bytes: 5 * 1024,
};

let current: Settings = { ...DEFAULTS };
const subs = new Set<(s: Settings) => void>();

export const getSettings = (): Settings => current;

export async function loadSettings(): Promise<Settings> {
  try {
    current = { ...DEFAULTS, ...(await api.getSettings()) };
  } catch {
    current = { ...DEFAULTS };
  }
  return current;
}

export async function updateSettings(patch: Partial<Settings>): Promise<Settings> {
  current = { ...DEFAULTS, ...(await api.setSettings(patch)) };
  subs.forEach((f) => f(current));
  return current;
}

export function onSettings(cb: (s: Settings) => void): () => void {
  subs.add(cb);
  return () => subs.delete(cb);
}
