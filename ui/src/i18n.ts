// Interface translation. English text is the key: t('Save') returns the English text unless the selected language
// has a catalog entry for it. Placeholders are {0}, {1} ... and are filled from the extra arguments.
// The language is read synchronously (before any module builds its UI), so a change is applied by reloading the page.
import de from './locales/de';
import es from './locales/es';
import fr from './locales/fr';
import ja from './locales/ja';
import ptBR from './locales/pt-BR';
import ru from './locales/ru';
import zhCN from './locales/zh-CN';

export const LANGUAGES: { code: string; name: string }[] = [
  { code: 'en', name: 'English' },
  { code: 'ru', name: 'Русский' },
  { code: 'es', name: 'Español' },
  { code: 'de', name: 'Deutsch' },
  { code: 'fr', name: 'Français' },
  { code: 'pt-BR', name: 'Português (Brasil)' },
  { code: 'zh-CN', name: '简体中文' },
  { code: 'ja', name: '日本語' },
];

const CATALOGS: Record<string, Record<string, string>> = { ru, es, de, fr, 'pt-BR': ptBR, 'zh-CN': zhCN, ja };
const CACHE_KEY = 'd256shell.lang';

/** "auto" or an unknown code -> the best supported language for this system. */
export function resolveLanguage(pref: string): string {
  if (LANGUAGES.some((l) => l.code === pref)) return pref;
  const sys = (navigator.language || 'en').toLowerCase();
  if (sys.startsWith('pt')) return 'pt-BR';
  if (sys.startsWith('zh')) return 'zh-CN';
  return LANGUAGES.find((l) => sys.startsWith(l.code.toLowerCase()))?.code ?? 'en';
}

function cached(): string {
  try {
    return localStorage.getItem(CACHE_KEY) ?? 'auto';
  } catch {
    return 'auto';
  }
}

let lang = resolveLanguage(cached());
let catalog: Record<string, string> = CATALOGS[lang] ?? {};
document.documentElement.lang = lang;

export const currentLanguage = (): string => lang;

/** Remember the preference (from settings.json). Returns true when the active language changed: the caller reloads. */
export function applyLanguage(pref: string): boolean {
  try {
    localStorage.setItem(CACHE_KEY, pref);
  } catch {
    /* storage can be unavailable: the preference is then applied on the next start from settings */
  }
  return resolveLanguage(pref) !== lang;
}

export function t(key: string, ...args: (string | number | undefined | null)[]): string {
  const s = catalog[key] ?? key;
  return args.length ? s.replace(/\{(\d+)\}/g, (m, i) => (+i >= args.length ? m : String(args[+i] ?? ""))) : s;
}

/** Number formatting and dates follow the interface language. */
export const locale = (): string => lang;

// Messages that come from the core are English with variable parts ("Could not read the key /a/b: denied").
// A catalog key may contain {0}, {1}: it then matches such a message as a pattern.
let patterns: { re: RegExp; to: string }[] | null = null;
export function tm(message: string): string {
  if (lang === 'en') return message;
  const direct = catalog[message];
  if (direct !== undefined) return direct;
  patterns ??= Object.entries(catalog)
    .filter(([k]) => /\{\d+\}/.test(k))
    .map(([k, to]) => ({
      re: new RegExp('^' + k.replace(/[.*+?^$()|[\]\\]/g, '\\$&').replace(/\{\d+\}/g, '(.+?)') + '$', 's'),
      to,
    }));
  for (const { re, to } of patterns) {
    const m = re.exec(message);
    if (m) return to.replace(/\{(\d+)\}/g, (x, i) => m[+i + 1] ?? x);
  }
  return message;
}
