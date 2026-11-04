// Update check against the public GitHub repository. One HTTPS request to api.github.com (the latest release); nothing about the
// user is sent, nothing is downloaded or installed: a newer version is only announced, with a link to its release page.
import { inTauri } from './api';
import { t } from './i18n';
import { pref } from './util';

declare const __APP_VERSION__: string;

export const REPO = 'dest1256/d256-Shell';
const API = `https://api.github.com/repos/${REPO}/releases/latest`;
const RELEASE_PREFIX = `https://github.com/${REPO}/`;
const DAY = 24 * 60 * 60 * 1000;

/** "v1.2.3" / "1.2.3-beta" -> [1, 2, 3]; null when it is not a version. */
export function parseVersion(tag: string): [number, number, number] | null {
  const m = /^v?(\d+)\.(\d+)\.(\d+)/.exec(tag.trim());
  return m ? [Number(m[1]), Number(m[2]), Number(m[3])] : null;
}

/** Is `a` a newer version than `b`? */
export function isNewer(a: string, b: string): boolean {
  const x = parseVersion(a);
  const y = parseVersion(b);
  if (!x || !y) return false;
  for (let i = 0; i < 3; i++) if (x[i] !== y[i]) return x[i] > y[i];
  return false;
}

export type UpdateResult =
  | { state: 'current'; latest: string }
  | { state: 'available'; latest: string; url: string }
  | { state: 'error'; message: string };

/** Ask GitHub for the latest published release (drafts and pre-releases are not returned by this endpoint). */
export async function fetchLatest(): Promise<UpdateResult> {
  const ctl = new AbortController();
  const timer = setTimeout(() => ctl.abort(), 8000);
  try {
    const r = await fetch(API, { headers: { Accept: 'application/vnd.github+json' }, signal: ctl.signal, credentials: 'omit', cache: 'no-store' });
    if (!r.ok) return { state: 'error', message: r.status === 404 ? t('No release was found.') : `HTTP ${r.status}` };
    const j = (await r.json()) as { tag_name?: string; html_url?: string };
    const tag = String(j.tag_name ?? '');
    if (!parseVersion(tag)) return { state: 'error', message: t('Unexpected answer from GitHub.') };
    const latest = tag.replace(/^v/, '');
    // only a page of this repository is ever opened
    const url = typeof j.html_url === 'string' && j.html_url.startsWith(RELEASE_PREFIX) ? j.html_url : `${RELEASE_PREFIX}releases/latest`;
    return isNewer(tag, __APP_VERSION__) ? { state: 'available', latest, url } : { state: 'current', latest };
  } catch (e) {
    return { state: 'error', message: e instanceof Error && e.name !== 'AbortError' ? e.message : t('no answer in 8 seconds') };
  } finally {
    clearTimeout(timer);
  }
}

/** The start-up check: at most once a day, and a version the user dismissed is not announced again. */
export async function autoCheck(): Promise<(UpdateResult & { state: 'available' }) | null> {
  const last = Number(pref.get('update-checked') ?? 0);
  if (Date.now() - last < DAY) return null;
  const r = await fetchLatest();
  if (r.state === 'error') return null; // silent: no network is normal
  pref.set('update-checked', String(Date.now()));
  if (r.state === 'available' && pref.get('update-skip') !== r.latest) return r;
  return null;
}

export const dismissUpdate = (version: string): void => pref.set('update-skip', version);

export async function openRelease(url: string): Promise<void> {
  if (!url.startsWith(RELEASE_PREFIX)) return;
  if (inTauri) {
    const { openUrl } = await import('@tauri-apps/plugin-opener');
    await openUrl(url);
  } else {
    window.open(url, '_blank', 'noopener,noreferrer');
  }
}
