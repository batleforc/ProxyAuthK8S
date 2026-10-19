// RFC 0002 §4.3: fails when a key is missing from en.json or fr.json, or when
// the code asks for a key neither declares. Orphan keys are listed, not fatal.
// Same idea as batlehub's build/i18n-audit.mjs, written for this front.
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';

const root = join(import.meta.dirname, '..', 'src');
const flat = (obj, prefix = '') =>
  Object.entries(obj).flatMap(([k, v]) =>
    typeof v === 'object' ? flat(v, `${prefix}${k}.`) : [`${prefix}${k}`],
  );
const en = new Set(
  flat(JSON.parse(readFileSync(join(root, 'i18n/en.json'), 'utf8'))),
);
const fr = new Set(
  flat(JSON.parse(readFileSync(join(root, 'i18n/fr.json'), 'utf8'))),
);

const files = (dir) =>
  readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return files(path);
    return /\.(vue|ts)$/.test(name) && !name.endsWith('.spec.ts') ? [path] : [];
  });

// Literal keys, and the static prefix of templated ones (`cluster.status.${s}`).
const used = new Set();
const prefixes = new Set();
for (const path of files(root)) {
  for (const [, key] of readFileSync(path, 'utf8').matchAll(
    /\bt\(\s*[`'"]([\w.${}-]+)[`'"]/g,
  )) {
    if (key.includes('${')) prefixes.add(key.slice(0, key.indexOf('${')));
    else used.add(key);
  }
}

const errors = [
  ...[...en].filter((k) => !fr.has(k)).map((k) => `missing in fr.json: ${k}`),
  ...[...fr].filter((k) => !en.has(k)).map((k) => `missing in en.json: ${k}`),
  ...[...used]
    .filter((k) => !en.has(k))
    .map((k) => `used but undeclared: ${k}`),
  ...[...prefixes]
    .filter((p) => ![...en].some((k) => k.startsWith(p)))
    .map((p) => `used prefix with no key: ${p}`),
];
const orphans = [...en].filter(
  (k) => !used.has(k) && ![...prefixes].some((p) => k.startsWith(p)),
);
if (orphans.length)
  console.log(`orphan keys (not fatal):\n  ${orphans.join('\n  ')}`);
if (errors.length) {
  console.error(errors.join('\n'));
  process.exit(1);
}
console.log(`i18n: ${en.size} keys, en and fr in step`);
