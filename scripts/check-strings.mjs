/**
 * scripts/check-strings.mjs — CI gate for the string table.
 *
 * Run: node scripts/check-strings.mjs        (exit 1 on any error)
 *      node scripts/check-strings.mjs --strict  (also fail on warnings)
 *
 * Without this, localisation rots quietly: a renamed id leaves a console
 * rendering ⟨some.id⟩, and a new hardcoded literal simply never gets
 * translated. Neither shows up in cargo test or the smoke suite.
 *
 * Errors (always fail):
 *   - duplicate or blank ids in strings.csv
 *   - a row whose parsed field count does not match the header — an unquoted
 *     delimiter (usually a comma) inside a value split it early, silently
 *     truncating player-visible text (issue #966)
 *   - a t('...') / data-i18n id with no CSV row
 *   - a localisable TOML key still holding prose instead of an id
 *
 * Warnings (fail only under --strict):
 *   - untranslated literals still in the client. These are reported rather
 *     than enforced while the client migration is in progress; flip this to
 *     an error once gui/ is fully migrated.
 */

import { readFile, readdir, stat } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { buildTable, parseCsv } from '../gui/strings.js';
import { rowLineNumbers } from './strings-csv.mjs';
import { isLocalisable } from './strings-rules.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const STRICT = process.argv.includes('--strict');

const errors = [];
const warnings = [];

/** Recursively list files under `dir` matching `test`. */
async function walk(dir, test, acc = []) {
  let entries;
  try { entries = await readdir(dir); } catch { return acc; }
  for (const name of entries) {
    if (name === 'node_modules' || name === 'dist' || name === 'target' || name === '.git') continue;
    const full = path.join(dir, name);
    const st = await stat(full);
    if (st.isDirectory()) await walk(full, test, acc);
    else if (test(full)) acc.push(full);
  }
  return acc;
}

const rel = (f) => path.relative(root, f).replace(/\\/g, '/');

// ── 1. CSV integrity ────────────────────────────────────────────────────────

const csvPath = path.join(root, 'assets', 'strings', 'strings.csv');
let csvText;
try {
  csvText = await readFile(csvPath, 'utf8');
} catch {
  console.error('assets/strings/strings.csv not found — run scripts/extract-strings.mjs');
  process.exit(1);
}

const rows = parseCsv(csvText);
const header = rows[0] ?? [];
for (const required of ['id', 'context', 'en']) {
  if (!header.includes(required)) errors.push(`strings.csv: missing '${required}' column`);
}

// The multi-line quoted values in the comms prose mean a row's index in `rows`
// is nowhere near its line in the file — 34 adrift by the end. Resolve the real
// line so an error points a human at the row it is actually complaining about,
// and fall back to the row number rather than name the wrong line.
const csvLines = rowLineNumbers(csvText, rows);
const at = (i) => (csvLines[i] === null ? `strings.csv row ${i + 1}` : `strings.csv:${csvLines[i]}`);

const idCol = header.indexOf('id');
const contextCol = header.indexOf('context');
const seen = new Set();
for (let i = 1; i < rows.length; i += 1) {
  const id = (rows[i][idCol] || '').trim();
  if (id === '') { errors.push(`${at(i)}: blank id`); continue; }
  if (seen.has(id)) errors.push(`${at(i)}: duplicate id '${id}'`);
  seen.add(id);
  if (rows[i].length !== header.length) {
    errors.push(
      `${at(i)}: '${id}' parses to ${rows[i].length} fields, expected ${header.length} — ` +
      'an unquoted delimiter inside a value is splitting it early and truncating the text',
    );
  }
  if ((rows[i][contextCol] || '').trim() === '') {
    warnings.push(`${at(i)}: '${id}' has no context — a translator cannot place it`);
  }
}

const table = buildTable(csvText);

// ── 2. Every referenced id exists ───────────────────────────────────────────

// gui/strings.js and gui/strings-boot.js are the machinery itself — their
// doc comments contain illustrative t('console.sensors.…') calls that are
// examples, not real lookups, and would otherwise be reported as missing.
const codeFiles = [
  ...await walk(
    path.join(root, 'gui'),
    (f) => /\.(js|html)$/.test(f) && !/[\\/]strings(-boot)?\.js$/.test(f),
  ),
  path.join(root, 'client.html'),
  path.join(root, 'server.html'),
];

// `t('some.id')` / `t("some.id")` — the trailing [,)] excludes computed ids
// like t('console.stance.' + stance), which cannot be checked statically.
const T_CALL = /\bt\(\s*(['"])([A-Za-z0-9_.-]+)\1\s*[,)]/g;
const DATA_I18N = /data-i18n\s*=\s*"([^"]+)"/g;
const DATA_I18N_ATTR = /data-i18n-attr\s*=\s*"([^"]+)"/g;

for (const file of codeFiles) {
  let src;
  try { src = await readFile(file, 'utf8'); } catch { continue; }

  for (const m of src.matchAll(T_CALL)) {
    if (!table.has(m[2])) errors.push(`${rel(file)}: t('${m[2]}') has no CSV row`);
  }
  for (const m of src.matchAll(DATA_I18N)) {
    if (!table.has(m[1])) errors.push(`${rel(file)}: data-i18n="${m[1]}" has no CSV row`);
  }
  for (const m of src.matchAll(DATA_I18N_ATTR)) {
    for (const pair of m[1].split(',')) {
      const id = pair.slice(pair.indexOf(':') + 1).trim();
      if (id && !table.has(id)) {
        errors.push(`${rel(file)}: data-i18n-attr id "${id}" has no CSV row`);
      }
    }
  }
}

// ── 3. TOML holds ids, not prose ────────────────────────────────────────────

// An id looks like `entity.foo.bar` — lowercase dotted, no spaces.
const LOOKS_LIKE_ID = /^[a-z][a-z0-9_]*(\.[a-z0-9_-]+)+$/;

for (const [dir, prefix] of [['entities', 'entity'], ['worlds', 'world'], ['factions', 'faction']]) {
  for (const file of await walk(path.join(root, 'assets', dir), (f) => f.endsWith('.toml'))) {
    const src = await readFile(file, 'utf8');
    let currentHeader = '';
    let lineNo = 0;

    for (const line of src.split('\n')) {
      lineNo += 1;
      const trimmed = line.trim();
      if (trimmed.startsWith('#')) continue;

      const head = trimmed.match(/^\[\[?([^\]]+)\]\]?$/);
      if (head) { currentHeader = head[1].trim(); continue; }

      const kv = trimmed.match(/^([A-Za-z0-9_-]+)\s*=\s*"([^"]*)"/);
      if (!kv) continue;
      const [, key, value] = kv;
      if (!isLocalisable(key, currentHeader, prefix)) continue;
      if (value === '') continue;

      if (!LOOKS_LIKE_ID.test(value)) {
        errors.push(`${rel(file)}:${lineNo}: ${key} holds prose, not a string id: "${value}"`);
      } else if (!table.has(value)) {
        errors.push(`${rel(file)}:${lineNo}: ${key} = "${value}" has no CSV row`);
      }
    }
  }
}

// ── 4. Untranslated literals still in the client (report-only for now) ──────

// Developer/operator-facing prose that deliberately stays English: crash
// guidance pointing at DevTools, and the host debug panel. Not player text.
const DEV_FACING = [
  'WASM trap (RuntimeError)',
];

const LITERAL_ASSIGN = /\.textContent\s*=\s*(['"])([^'"]*[A-Za-z]{3}[^'"]*)\1/g;
// Skip strings that are obviously not prose: css values, ids, single tokens
// used as keys. A literal is interesting if it contains a space or is
// all-caps display text.
const INTERESTING = (s) => /\s/.test(s) || /^[A-Z][A-Z ]{2,}$/.test(s);

for (const file of codeFiles) {
  let src;
  try { src = await readFile(file, 'utf8'); } catch { continue; }
  for (const m of src.matchAll(LITERAL_ASSIGN)) {
    if (INTERESTING(m[2]) && !DEV_FACING.some((d) => m[2].includes(d))) {
      warnings.push(`${rel(file)}: hardcoded textContent "${m[2]}" — not localised`);
    }
  }
}

// ── Report ──────────────────────────────────────────────────────────────────

for (const w of warnings) console.warn(`warn  ${w}`);
for (const e of errors) console.error(`error ${e}`);

console.log(
  `\n${table.size} strings; ${errors.length} error(s), ${warnings.length} warning(s)`,
);

if (errors.length > 0 || (STRICT && warnings.length > 0)) process.exit(1);
