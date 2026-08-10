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
 *   - a DERIVED id with no CSV row: one the client composes at runtime from a
 *     machine identifier in the TOML, so it is spelled out nowhere for the two
 *     checks above to find (issue #949)
 *
 * Warnings (fail only under --strict):
 *   - untranslated literals still in the client. These are reported rather
 *     than enforced while the client migration is in progress; flip this to
 *     an error once gui/ is fully migrated. Three shapes are scanned:
 *     `.textContent =` assignments, text nodes in markup, and text-bearing
 *     attributes — the last two both in `.html` files and in the template
 *     literals a web component builds its shadow DOM from, which rendered
 *     hardcoded English with this gate green until issue #976.
 *   - a `data-i18n` in a JS-built template. `applyToDom` runs over `document`
 *     at boot and never on a shadowRoot, so the tag resolves nothing while
 *     still exempting its subtree from the scan above.
 *   - a region of JS the markup scanner could not lex. Reported, never
 *     swallowed: "I stopped reading" must not look like "I read it and it was
 *     clean", which is the failure mode #976 was filed about.
 *
 * `docs/strings-authoring-guide.md` carries the inventory of what these scans
 * do and do not see. Keep it in step with any rule change here.
 */

import { readFile, readdir, stat } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { buildTable, parseCsv } from '../gui/strings.js';
import { rowLineNumbers } from './strings-csv.mjs';
import { untranslatedTextContent } from './strings-literals.mjs';
import { lineOf, untranslatedMarkup } from './strings-markup.mjs';
import { isLocalisable } from './strings-rules.mjs';
import { proseLiterals } from './strings-rust.mjs';
import { resolveThroughIncludes } from './toml-includes.mjs';

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
// Both quote styles, because the markup scan exempts both: `parseAttributes`
// in strings-markup.mjs reads `data-i18n='…'` and falls silent over the text
// under it. A double-quote-only id check here would leave a single-quoted typo
// exempt from BOTH scans — gate green, console rendering ⟨typo.id⟩.
const DATA_I18N = /data-i18n\s*=\s*(?:"([^"]+)"|'([^']+)')/g;
const DATA_I18N_ATTR = /data-i18n-attr\s*=\s*(?:"([^"]+)"|'([^']+)')/g;

for (const file of codeFiles) {
  let src;
  try { src = await readFile(file, 'utf8'); } catch { continue; }

  for (const m of src.matchAll(T_CALL)) {
    if (!table.has(m[2])) errors.push(`${rel(file)}: t('${m[2]}') has no CSV row`);
  }
  for (const m of src.matchAll(DATA_I18N)) {
    const id = m[1] ?? m[2];
    if (!table.has(id)) errors.push(`${rel(file)}: data-i18n="${id}" has no CSV row`);
  }
  for (const m of src.matchAll(DATA_I18N_ATTR)) {
    for (const pair of (m[1] ?? m[2]).split(',')) {
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

/**
 * Every `key = "value"` in a TOML source, tagged with the table header it sits
 * under ('' at top level) and its 1-based line.
 * @param {string} src
 */
function* tomlPairs(src) {
  let header = '';
  let lineNo = 0;
  for (const line of src.split('\n')) {
    lineNo += 1;
    const trimmed = line.trim();
    if (trimmed.startsWith('#')) continue;

    const head = trimmed.match(/^\[\[?([^\]]+)\]\]?$/);
    if (head) { header = head[1].trim(); continue; }

    const kv = trimmed.match(/^([A-Za-z0-9_-]+)\s*=\s*"([^"]*)"/);
    if (!kv) continue;
    yield { header, key: kv[1], value: kv[2], lineNo };
  }
}

const isToml = (f) => f.endsWith('.toml');
const entityFiles = await walk(path.join(root, 'assets', 'entities'), isToml);
const worldFiles = await walk(path.join(root, 'assets', 'worlds'), isToml);
const factionFiles = await walk(path.join(root, 'assets', 'factions'), isToml);
// The scenario manifests are shipped data with a localisable `[[scenario]]
// label` too, but they sit at the assets root rather than in one of the three
// directories above — so until #949 nothing looked at them at all.
const manifestFiles = (await readdir(path.join(root, 'assets')))
  .filter((name) => /^scenarios.*\.toml$/.test(name))
  .map((name) => path.join(root, 'assets', name));

const localisableToml = [
  ...entityFiles.map((f) => [f, 'entity']),
  ...worldFiles.map((f) => [f, 'world']),
  ...factionFiles.map((f) => [f, 'faction']),
  ...manifestFiles.map((f) => [f, 'world']),
];

for (const [file, prefix] of localisableToml) {
  const src = await readFile(file, 'utf8');
  for (const { header, key, value, lineNo } of tomlPairs(src)) {
    if (!isLocalisable(key, header, prefix)) continue;
    if (value === '') continue;

    if (!LOOKS_LIKE_ID.test(value)) {
      errors.push(`${rel(file)}:${lineNo}: ${key} holds prose, not a string id: "${value}"`);
    } else if (!table.has(value)) {
      errors.push(`${rel(file)}:${lineNo}: ${key} = "${value}" has no CSV row`);
    }
  }
}

// ── 3b. Ids the client COMPOSES from TOML data ──────────────────────────────

// Checks 2 and 3 both need an id spelled out somewhere — as a literal inside a
// t() call, or as a TOML value. A third class is spelled out nowhere: the
// client builds the id at runtime by interpolating a value the TOML authors as
// a machine IDENTIFIER, not as display text.
//
// `station.<id>.name` is the plain case. Station ids stay English in TOML
// because Rust matches stations by name (see strings-rules.mjs), so the display
// name is looked up from an id derived in gui/console-state.js instead. Neither
// half of that id is a literal anywhere, so a hull authoring
// `[[station]] id = "ops"` shipped a manual tab reading ⟨station.ops.name⟩ with
// every gate green — the reported symptom of issue #949.
//
// Derive the same ids the client will, from the same data, and hold them to the
// same "must have a row" rule.

/** @param {string} id @param {string} where @param {string} why */
const requireDerived = (id, where, why) => {
  if (!table.has(id)) errors.push(`${where}: ${why} derives '${id}', which has no CSV row`);
};

for (const file of entityFiles) {
  const src = await readFile(file, 'utf8');
  for (const { header, key, value, lineNo } of tomlPairs(src)) {
    if (value === '') continue;
    if (header === 'station' && key === 'id') {
      requireDerived(
        `station.${value}.name`,
        `${rel(file)}:${lineNo}`,
        'stationDisplayName (gui/console-state.js)',
      );
    } else if (header === 'station.rating' && key === 'name') {
      requireDerived(
        `station.rating.${value.toLowerCase()}.name`,
        `${rel(file)}:${lineNo}`,
        'the station-rating caption (gui/settings-panel.js, gui/manual-panel.js)',
      );
    }
  }
}

// The lobby ship picker badges a hull with its resolved `class` token and
// composes the caption id from it. Only hulls a world actually OFFERS can
// reach that badge, so the sweep starts from `[[available_ships]]` rather than
// from every entity template — an NPC-only hull never renders a class badge
// and must not be made to carry a caption for one.
const offeredHulls = new Set();
for (const file of worldFiles) {
  const src = await readFile(file, 'utf8');
  for (const { header, key, value } of tomlPairs(src)) {
    if (header === 'available_ships' && key === 'template_path') offeredHulls.add(value);
  }
}

// `class` is INHERITABLE: a hull that declares none takes its included
// fragment's (src/entities/include_resolve.rs), and that resolved value is what
// reaches the badge — the catalog entry reads `cfg.class` off the fully
// resolved EntityConfig (src/server/bridge.rs:1581). Reading only a hull's own
// top-level `class` would leave a composed hull at zero errors while the picker
// badged it with a raw token, so the closure is walked instead.
const readOrNull = async (file) => {
  try { return await readFile(file, 'utf8'); } catch { return null; }
};

for (const template of [...offeredHulls].sort()) {
  // Forward slashes throughout: resolveInclude normalises the paths it builds,
  // and `inherited` below compares one against the hull's own path.
  const file = path.join(root, template).replace(/\\/g, '/');
  // A template path that does not resolve is world::validate's finding to
  // report, not this gate's — say nothing rather than blame the string table.
  const found = await resolveThroughIncludes(file, 'class', readOrNull);
  if (!found || found.value === '') continue;
  const inherited = found.file !== file;
  requireDerived(
    `component.ship_picker.class.${found.value.toLowerCase()}`,
    `${rel(found.file)}:${found.lineNo}`,
    inherited
      ? `the lobby ship-picker class badge (gui/components/ph-ship-picker.js), ` +
        `via the class ${rel(file)} inherits from here`
      : 'the lobby ship-picker class badge (gui/components/ph-ship-picker.js)',
  );
}

// ── 4. Untranslated literals still in the client (report-only for now) ──────

// Developer/operator-facing prose that deliberately stays English: crash
// guidance pointing at DevTools, and the host debug panel. Not player text.
//
// Keyed BY FILE, not global. A bare substring list applies to every file in the
// sweep, so a future console rendering the words "WASM PANIC" to a player would
// be exempted by an allowance written for the server's crash overlay. An
// exemption should only reach the surface it was argued for.
const DEV_FACING = new Map([
  ['server.html', [
    'WASM trap (RuntimeError)',
    // The rest of the same panic overlay: the banner above the trace and the
    // "open DevTools, then reload" instruction under it. An operator reads this
    // after the server process has died; a player never sees it, because there
    // is nothing left rendering a console.
    'WASM PANIC',
    'The full stack trace is in the browser DevTools console',
  ]],
]);

// The `src/` half of the same rule lives in section 5 below (issue #975).

// Files whose English is not worth a lookup. Kept as a list of exact paths, not
// a pattern, so adding one is a visible decision in the diff.
const UNLOCALISED_FILES = new Set([
  // A four-line stub whose only job is `location.replace` for stale bookmarks.
  // Its <title> shows for the duration of one redirect; wiring the string table
  // into it would cost a fetch to translate a flicker.
  'gui/lobby-client.html',
]);

// The rules live in scripts/strings-literals.mjs and scripts/strings-markup.mjs
// so their edge cases (ternaries, fallbacks, template segments, `===` token
// tests, machine tokens in attributes) are unit-tested rather than only
// exercised by whatever happens to be in the client today.
for (const file of codeFiles) {
  let src;
  try { src = await readFile(file, 'utf8'); } catch { continue; }
  if (UNLOCALISED_FILES.has(rel(file))) continue;

  const devFacing = DEV_FACING.get(rel(file)) ?? [];

  for (const text of untranslatedTextContent(src)) {
    if (devFacing.some((d) => text.includes(d))) continue;
    warnings.push(`${rel(file)}: hardcoded textContent "${text}" — not localised`);
  }

  for (const found of untranslatedMarkup(src, file.endsWith('.html'))) {
    const where = `${rel(file)}:${lineOf(src, found.index)}`;

    // A region the JS lexer could not read is reported like any other finding.
    // Silence here would be indistinguishable from a clean file, and the whole
    // value of this gate is that its green means something.
    if (found.kind === 'unscannable') {
      warnings.push(
        `${where}: ${found.text} — the markup scan gave up here, so this region is UNCHECKED`,
      );
      continue;
    }

    if (devFacing.some((d) => found.text.includes(d))) continue;

    // `text` may carry the blanked width of an interpolation; a translator
    // reading a CI log wants the words, not the padding.
    const shown = found.text.replace(/\s+/g, ' ');

    if (found.kind === 'inert-i18n') {
      warnings.push(
        `${where}: ${found.attr}="${shown}" sits in a JS-built template, where nothing ` +
        'applies it — applyToDom runs over `document` at boot only, never on a shadowRoot ' +
        "or on markup built later. Use ${t('id')} in the template instead",
      );
      continue;
    }

    const what = found.kind === 'attr'
      ? `hardcoded ${found.attr}="${shown}"`
      : `hardcoded markup text "${shown}"`;
    warnings.push(`${where}: ${what} — not localised`);
  }
}

// ── 5. English composed in Rust on a wire-visible path (issue #975) ─────────

// AGENTS.md rule 11 forbids hardcoded player-visible English; until #975 nothing
// here read `src/`, so a `format!`/`.to_string()` that built display text and
// crossed a host channel shipped as English with this gate green. The rule that
// catches it lives in scripts/strings-rust.mjs and is deliberately NARROW: it
// runs over a short allowlist of modules known to compose wire-visible text —
// not all of `src/`, which would be a false-positive mine of log lines and
// machine tokens — and within each file reads only the production region and
// flags only prose (see that file for the exact signal). An ERROR, not a
// warning: these modules are fully migrated, so any prose here is a regression.
//
// Issue #977 closed the remaining wire-visible producers: the power group and
// shield facing labels, the navigation waypoint label, the intent-advisory
// subjects and the game-over reason all now emit `strings.csv` ids that
// `localiseTree` resolves client-side, so their modules join the list. Every
// prose literal in each file's production region — including incidental log and
// panic text — is now an error, which is the point: these modules are fully
// migrated and any new prose is a regression.
const WIRE_VISIBLE_RUST = [
  'src/ship/coordination_systems.rs',
  'src/ship/power.rs',
  'src/weapons/shield.rs',
  'src/console/navigation/mod.rs',
  'src/server/viewscreen_border.rs',
];

for (const relPath of WIRE_VISIBLE_RUST) {
  let src;
  try {
    src = await readFile(path.join(root, ...relPath.split('/')), 'utf8');
  } catch {
    errors.push(`${relPath}: listed in WIRE_VISIBLE_RUST but could not be read`);
    continue;
  }
  for (const { line, text } of proseLiterals(src)) {
    errors.push(
      `${relPath}:${line}: player-visible English composed in Rust: "${text}" — ` +
      'emit a string id (and params) the client resolves through strings.csv, ' +
      'not a composed sentence (AGENTS.md rule 11)',
    );
  }
}

// ── Report ──────────────────────────────────────────────────────────────────

for (const w of warnings) console.warn(`warn  ${w}`);
for (const e of errors) console.error(`error ${e}`);

console.log(
  `\n${table.size} strings; ${errors.length} error(s), ${warnings.length} warning(s)`,
);

if (errors.length > 0 || (STRICT && warnings.length > 0)) process.exit(1);
