// The only way the UI talks to the backend. Transport is chosen at runtime:
// Tauri `invoke` inside the app, HTTP JSON-RPC against devbridge in a plain browser (e2e tests).
import { Channel, invoke } from '@tauri-apps/api/core';

export class ApiError extends Error {
  constructor(
    public code: string,
    message: string,
    public details?: Record<string, unknown>,
  ) {
    super(message);
  }
}

export interface Prompt {
  id: number;
  kind: 'host_key' | 'input';
  // host_key
  host?: string;
  port?: number;
  key_type?: string;
  fingerprint?: string;
  // input
  text?: string;
  echo?: boolean;
  is_password?: boolean;
  saved_password_rejected?: boolean;
}

export interface ConnectParams {
  attempt: string;
  server_id?: string;
  host?: string;
  port?: number;
  username?: string;
  password?: string;
  save_password?: boolean;
  test_only?: boolean;
  key_path?: string;
  key_passphrase?: string;
}

export interface Connected {
  session_id: string;
  /** null when the server has no SFTP (the terminal still works). */
  home: string | null;
  sftp_error: string | null;
  name: string;
  host: string;
  port: number;
  username: string;
}

export interface DirEntry {
  name: string;
  kind: 'file' | 'dir' | 'symlink' | 'other';
  target_is_dir: boolean | null;
  size: number;
  mtime: number | null;
  mode: number | null;
  uid: number | null;
  gid: number | null;
  owner: string | null;
  group: string | null;
  name_lossy: boolean;
}

export interface Listing {
  path: string;
  entries: DirEntry[];
}

export interface ServerView {
  id: string;
  name: string;
  host: string;
  port: number;
  username: string;
  initial_dir: string | null;
  ssh_extra_args: string | null;
  has_password: boolean;
  key_path: string | null;
  has_key_passphrase: boolean;
}

export interface ServerInput {
  id?: string;
  name: string;
  host: string;
  port: number;
  username: string;
  password?: string; // omitted = keep stored password
  initial_dir?: string;
  ssh_extra_args?: string;
  /** Private key file for key login; empty = no key. */
  key_path?: string;
  /** omitted = keep the stored passphrase, empty = clear it */
  key_passphrase?: string;
}

export interface Settings {
  ssh_path: string;
  auto_password: 'auto' | 'askpass' | 'expect' | 'off';
  font_family: string;
  font_size: number;
  scheme: string;
  cursor_style: 'bar' | 'block' | 'underline';
  scrollback: number;
  right_click: 'menu' | 'copy-paste';
  wheel_mode: 'arrows' | 'mouse';
  parallel_transfers: number;
  keep_mtime: boolean;
  drag_out_limit_mb: number;
  editor_limit_mb: number;
  auto_lock_minutes: number;
  clone_mode: 'ask' | 'auto' | 'view';
  monitor_enabled: boolean;
  monitor_interval_ms: number;
  paste_warn_lines: number;
  paste_warn_bytes: number;
}

export interface SshInfo {
  found: boolean;
  error?: string;
  path?: string;
  version?: string;
  askpass_supported?: boolean;
  mode?: 'askpass' | 'expect' | 'off';
}

export interface TermOpened {
  term_id: string;
  mode: 'askpass' | 'expect' | 'off';
  ssh_version: string;
}

export interface Status {
  state: 'no_vault' | 'locked' | 'unlocked';
  backup_exists: boolean;
}

export const inTauri = '__TAURI_INTERNALS__' in window;

async function call<T>(method: string, params: Record<string, unknown> = {}): Promise<T> {
  if (inTauri) {
    try {
      return (await invoke('api', { method, params })) as T;
    } catch (e) {
      const err = e as { code?: string; message?: string; details?: Record<string, unknown> };
      throw new ApiError(err.code ?? 'internal', err.message ?? String(e), err.details);
    }
  }
  const r = await fetch('/rpc', { method: 'POST', body: JSON.stringify({ method, params }) });
  const j = await r.json();
  if (!j.ok) throw new ApiError(j.error.code, j.error.message, j.error.details);
  return j.result as T;
}

export const api = {
  status: () => call<Status>('status'),
  createVault: (master: string) => call<Status>('create_vault', { master }),
  unlock: (master: string) => call<Status>('unlock', { master }),
  restoreBackup: (master: string) => call<Status>('restore_backup', { master }),
  lock: () => call<Status>('lock'),
  changeMaster: (old: string, nw: string) => call<object>('change_master', { old, new: nw }),
  listServers: () => call<ServerView[]>('list_servers'),
  getPassword: (id: string) => call<string | null>('get_password', { id }),
  saveServer: (s: ServerInput) => call<{ id: string }>('save_server', { ...s }),
  duplicateServer: (id: string) => call<{ id: string } | null>('duplicate_server', { id }),
  deleteServer: (id: string) => call<object>('delete_server', { id }),
  connect: (p: ConnectParams) => call<Connected>('connect', { ...p }),
  pollPrompts: (attempt: string) => call<Prompt[]>('poll_prompts', { attempt }),
  answerPrompt: (id: number, a: { accept?: boolean; value?: string }) => call<object>('answer_prompt', { id, ...a }),
  cancelConnect: (attempt: string) => call<object>('cancel_connect', { attempt }),
  replaceHostKey: (attempt: string) => call<object>('replace_host_key', { attempt }),
  listDir: (session_id: string, path?: string) => call<Listing>('list_dir', { session_id, path }),
  fs: <T = Record<string, never>>(session_id: string, op: string, params: Record<string, unknown> = {}) => call<T>('fs', { session_id, op, ...params }),
  disconnect: (session_id: string) => call<object>('disconnect', { session_id }),
  getSettings: () => call<Settings>('get_settings'),
  setSettings: (patch: Partial<Settings>) => call<Settings>('set_settings', { ...patch }),
  sshInfo: () => call<SshInfo>('ssh_info'),
  termContext: (term_id: string) => call<TermContext>('term_context', { term_id }),
  monitorSample: (session_id: string, info = false) => call<Record<string, unknown>>('monitor_sample', { session_id, info }),
  setServerPassword: (id: string, password: string) => call<object>('set_server_password', { id, password }),
  termOpen: (session_id: string, cols: number, rows: number) => call<TermOpened>('term_open', { session_id, cols, rows }),
  termWrite: (term_id: string, bytes: Uint8Array) => call<object>('term_write', { term_id, data_b64: toBase64(bytes) }),
  termResize: (term_id: string, cols: number, rows: number) => call<object>('term_resize', { term_id, cols, rows }),
  termClose: (term_id: string) => call<object>('term_close', { term_id }),
};

function toBase64(b: Uint8Array): string {
  let s = '';
  for (let i = 0; i < b.length; i += 0x8000) s += String.fromCharCode(...b.subarray(i, i + 0x8000));
  return btoa(s);
}

function fromBase64(s: string): Uint8Array {
  const bin = atob(s);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

/**
 * Receive terminal output. In the app: raw bytes over a Tauri channel. In a browser (e2e via devbridge):
 * long polling. Returns a function that stops the stream.
 */
export function termStream(termId: string, onData: (b: Uint8Array) => void, onExit: (code: number) => void): () => void {
  let stopped = false;
  if (inTauri) {
    const ch = new Channel<ArrayBuffer | { exit: number }>();
    ch.onmessage = (m) => {
      if (stopped) return;
      if (m instanceof ArrayBuffer) onData(new Uint8Array(m));
      else if (m && typeof (m as { exit?: number }).exit === 'number') onExit((m as { exit: number }).exit);
    };
    void invoke('term_stream', { termId, onData: ch }).catch(() => undefined);
    return () => {
      stopped = true;
    };
  }
  void (async () => {
    while (!stopped) {
      try {
        const r = await call<{ data: string | null; exit: number | null }>('term_read', { term_id: termId, wait_ms: 300 });
        if (r.data) onData(fromBase64(r.data));
        if (r.exit !== null) {
          onExit(r.exit);
          return;
        }
      } catch {
        return; // terminal closed
      }
    }
  })();
  return () => {
    stopped = true;
  };
}

export interface TransferView {
  id: string;
  kind: 'upload' | 'download';
  label: string;
  state: 'queued' | 'running' | 'done' | 'error' | 'cancelled';
  done: number;
  total: number;
  speed: number;
  current: string;
  error_code: string | null;
  error: string | null;
  files: number;
  skipped: string[];
  dest: string;
  session_id: string;
  item_paths: string[];
}

export interface PlanItem {
  path: string;
  name: string;
  exists: boolean;
  is_dir: boolean;
}

export type ConflictAction = 'replace' | 'skip' | 'rename';

export interface EditBase {
  sha256: string;
  size: number;
  mtime: number | null;
  mode: number | null;
  uid: number | null;
}

export interface EditOpened {
  path: string;
  text: string;
  bom: boolean;
  eol: 'lf' | 'crlf' | 'mixed';
  /** Encoding the file was read in (WHATWG name) and whether it was only guessed. */
  encoding: string;
  guessed: boolean;
  base: EditBase | null;
  new: boolean;
}

export interface EditSaveOpts {
  path: string;
  text: string;
  bom: boolean;
  eol: 'lf' | 'crlf';
  encoding: string;
  base: EditBase | null;
  force?: boolean;
  allow_foreign?: boolean;
  in_place?: boolean;
}

export const transfers = {
  plan: (session_id: string, kind: 'upload' | 'download', paths: string[], dest: string) => call<PlanItem[]>('transfer_plan', { session_id, kind, paths, dest }),
  start: (p: { session_id: string; kind: 'upload' | 'download'; items: { path: string; action: ConflictAction }[]; dest?: string; stage?: boolean }) => call<{ id: string; dest: string }>('transfer_start', { ...p }),
  list: () => call<TransferView[]>('transfer_list'),
  cancel: (id: string) => call<object>('transfer_cancel', { id }),
  retry: (id: string, o?: { session_id?: string; allow_foreign?: boolean; in_place?: boolean }) => call<{ id: string; dest: string }>('transfer_retry', { id, ...o }),
  clear: () => call<object>('transfer_clear'),
  stageClear: (path: string) => call<object>('stage_clear', { path }),
};

export const editor = {
  open: (session_id: string, path: string, isNew = false, encoding?: string) => call<EditOpened>('edit_open', { session_id, path, new: isNew, encoding }),
  save: (session_id: string, o: EditSaveOpts) => call<{ base: EditBase }>('edit_save', { session_id, ...o }),
};

export interface Native {
  /** Native file / folder picker (in a browser: a text prompt, used by the e2e tests). */
  pick(kind: 'files' | 'folder'): Promise<string[]>;
  /** Native drag of staged files into Explorer (Tauri only). */
  dragOut(paths: string[]): Promise<void>;
  /** Files dropped from Windows Explorer onto the window; positions are CSS pixels. */
  onFileDrop(handlers: { enter: (x: number, y: number) => void; over: (x: number, y: number) => void; leave: () => void; drop: (paths: string[], x: number, y: number) => void }): Promise<() => void>;
  readonly isTauri: boolean;
}

/** What the shell of a terminal is doing, found on the server (`/proc` over SFTP). */
export interface TermContext {
  found: boolean;
  reason?: string;
  shell?: string;
  cwd?: string | null;
  /** argv of the foreground program (null at an idle prompt). */
  fg?: string[] | null;
  fg_comm?: string | null;
  env?: Record<string, string>;
  /** The login shell runs as root (so `su - user` needs no password). */
  root?: boolean;
  /** The user went deeper (su, sudo -i, bash ...): the command that started the nested shell. */
  nested?: { launcher: string[]; shell: string } | null;
}
