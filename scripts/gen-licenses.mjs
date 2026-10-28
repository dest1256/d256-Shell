// Builds ui/src/licenses.txt: licence texts of every library that ends up in the app (Rust via cargo-about,
// npm via the installed production dependencies). Run in the dev container: node scripts/gen-licenses.mjs
import { execFileSync } from 'node:child_process';
import { existsSync, readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';

const root = resolve(new URL('..', import.meta.url).pathname);
const out = [];
out.push('d256 Shell — лицензии используемых открытых библиотек.\nd256 Shell © 2026 dest1256. Библиотеки ниже распространяются на условиях своих лицензий.\n');

out.push('\n######## Rust ########\n');
out.push(execFileSync('cargo', ['about', 'generate', '--locked', 'about.hbs'], { cwd: root, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024, stdio: ['ignore', 'pipe', 'ignore'] }));

out.push('\n######## npm (production) ########\n');
const ui = join(root, 'ui');
const dirs = execFileSync('npm', ['ls', '--omit=dev', '--all', '--parseable'], { cwd: ui, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 }).split('\n').map((s) => s.trim()).filter((p) => p && p !== ui);
const seen = new Set();
for (const d of [...new Set(dirs)].sort()) {
  const pj = join(d, 'package.json');
  if (!existsSync(pj)) continue;
  const p = JSON.parse(readFileSync(pj, 'utf8'));
  const id = `${p.name}@${p.version}`;
  if (seen.has(id)) continue;
  seen.add(id);
  const file = readdirSync(d).find((f) => /^(licen[cs]e|copying)/i.test(f));
  const lic = typeof p.license === 'string' ? p.license : JSON.stringify(p.license ?? p.licenses ?? 'см. package.json');
  const repo = typeof p.repository === 'string' ? p.repository : (p.repository?.url ?? '');
  out.push(`\n================================================================================\n${id}  —  ${lic}${repo ? `\n${repo}` : ''}\n`);
  out.push(file ? readFileSync(join(d, file), 'utf8') : `(файл лицензии в пакете не найден; лицензия: ${lic})`);
}
writeFileSync(join(ui, 'src', 'licenses.txt'), out.join('\n'));
console.log(`licenses.txt: ${seen.size} npm packages`);
