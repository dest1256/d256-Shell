// Every t('...') / tr('...') key used by the UI must exist in every language catalog with the same {n} placeholders.
import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(fileURLToPath(import.meta.url), '..', '..');
const src = join(root, 'ui', 'src');
const langs = ['ru', 'es', 'de', 'fr', 'pt-BR', 'zh-CN', 'ja'];
const unesc = (s) => s.replace(/\\(.)/g, (m, c) => (c === 'n' ? '\n' : c === 't' ? '\t' : c));
const ph = (s) => (s.match(/\{\d+\}/g) ?? []).sort().join(',');

const keys = new Set();
for (const f of readdirSync(src).filter((f) => f.endsWith('.ts'))) {
  if (f === 'i18n.ts') continue;
  const text = readFileSync(join(src, f), 'utf8');
  for (const m of text.matchAll(/(?<![A-Za-z0-9_$.])tr?\(\s*(?:'((?:[^'\\]|\\.)*)'|"((?:[^"\\]|\\.)*)")/g)) keys.add(unesc(m[1] ?? m[2]));
}

let problems = 0;
for (const lang of langs) {
  const text = readFileSync(join(src, 'locales', lang + '.ts'), 'utf8');
  const body = text.slice(text.indexOf('{\n') + 1, text.lastIndexOf('};'));
  const cat = JSON.parse('{' + body.trim().replace(/,\s*$/, '') + '}');
  let missing = 0;
  for (const k of keys) {
    if (!(k in cat)) { missing++; problems++; console.log(`${lang}: missing "${k.slice(0, 80)}"`); }
    else if (ph(k) !== ph(cat[k])) { problems++; console.log(`${lang}: placeholders differ in "${k.slice(0, 60)}"`); }
  }
  console.log(`${lang}: ${Object.keys(cat).length} entries, ${missing} UI keys missing`);
}
console.log(`i18n: ${keys.size} UI keys checked in ${langs.length} languages, ${problems} problems`);
process.exit(problems ? 1 : 0);
