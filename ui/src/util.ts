import { ApiError } from './api';
import { h } from './dom';

export function errText(e: unknown): string {
  return e instanceof ApiError ? e.message : String(e);
}

export function pwField(id: string, placeholder: string, autocomplete = 'current-password'): HTMLInputElement {
  return h('input', { type: 'password', id, placeholder, autocomplete, 'data-testid': id }) as HTMLInputElement;
}

/** Per-viewer UI convenience only; never throws (storage can be unavailable). */
export const pref = {
  get(key: string): string | null {
    try {
      return localStorage.getItem('d256shell.' + key);
    } catch {
      return null;
    }
  },
  set(key: string, value: string): void {
    try {
      localStorage.setItem('d256shell.' + key, value);
    } catch {
      /* ignore */
    }
  },
};
