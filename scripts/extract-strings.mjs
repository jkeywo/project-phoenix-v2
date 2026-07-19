/**
 * scripts/extract-strings.mjs — One-shot migration of TOML display text into
 * assets/strings/strings.csv, rewriting the TOML to hold string ids.
 *
 * Run once:  node scripts/extract-strings.mjs
 * Preview:   node scripts/extract-strings.mjs --dry-run
 *
 * What it does
 * ------------
 * Walks assets/{entities,worlds,factions}/*.toml, pulls every localisable value
 * into a CSV row, and replaces the value in-place with the row's id. English
 * text is wrapped in [square brackets] to mark it as agent-drafted placeholder
 * copy awaiting human review.
 *
 * Why line-based and not parse-then-serialise
 * -------------------------------------------
 * These TOML files carry a lot of hand-written comments and box-drawing section
 * headers that a round-trip through a TOML serialiser would flatten. We rewrite
 * the source text line by line so everything except the targeted string values
 * survives byte-for-byte.
 *
 * The entity-name problem
 * -----------------------
 * In assets/worlds/*.toml, `[[entity]] name` is dual-purpose: it is display text
 * AND the cross-reference key that `entity = "..."` and `targets = [...]` point
 * at, which Rust resolves through `DispatchContext::name_to_uuid`
 * (src/world/dispatch.rs:112). Rust treats it as an opaque string, so swapping
 * the name for an id is safe *provided every reference is swapped to the same
 * id*. We therefore run in two passes: collect all entity names first, then
 * rewrite definitions and references together. A reference is only rewritten
 * when its value exactly matches a collected entity name — that keeps us off
 * `anchor = "ironveil_patrol_b"` and other same-shaped keys that are not names.
 */

import { readFile, writeFile, mkdir, readdir } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { REFERENCE_KEYS, isLocalisable } from './strings-rules.mjs';
import { parseCsv } from '../gui/strings.js';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const DRY_RUN = process.argv.includes('--dry-run');

// Fields that make a better id fragment than an array index, most specific
// first. `[[hull.system_hull]]` gets `.power-reactor`, not `.0`.
const DISCRIMINATORS = ['system_id', 'id', 'thread_id', 'group_id', 'slot'];

// A value that already looks like a string id has been migrated already.
// Re-running must skip it, or a second pass would treat the id as prose and
// mint `entity.foo.name` → `entity.entity_foo_name.name`. Display text never
// takes this shape: lowercase, dotted, no spaces.
const LOOKS_LIKE_ID = /^[a-z][a-z0-9_]*(\.[a-z0-9_-]+)+$/;

// ── Small TOML-aware line walker ────────────────────────────────────────────

/**
 * Split TOML source into logical units: table headers, key/value pairs (single
 * or multi-line), and everything else passed through untouched.
 *
 * @param {string} src
 * @returns {Array<{kind: string, lines: string[], key?: string, value?: string, header?: string, array?: boolean}>}
 */
function tokenise(src) {
  const lines = src.split('\n');
  const out = [];
  let i = 0;

  while (i < lines.length) {
    const line = lines[i];
    const trimmed = line.trim();

    const header = trimmed.match(/^\[(\[?)([^\]]+)\]\]?$/);
    if (header) {
      out.push({ kind: 'header', lines: [line], header: header[2].trim(), array: header[1] === '[' });
      i += 1;
      continue;
    }

    const kv = trimmed.match(/^([A-Za-z0-9_-]+)\s*=\s*(.*)$/);
    if (kv && !trimmed.startsWith('#')) {
      const key = kv[1];
      let rest = kv[2];
      const block = [line];

      if (rest.startsWith('"""')) {
        // Multi-line basic string: consume until the closing delimiter.
        while (!(block.join('\n').slice(block[0].indexOf('"""') + 3).includes('"""'))) {
          i += 1;
          if (i >= lines.length) break;
          block.push(lines[i]);
        }
        out.push({ kind: 'kv', lines: block, key, value: null, multiline: true });
        i += 1;
        continue;
      }

      if (rest.startsWith('[') && !rest.includes(']')) {
        // Array spanning lines.
        while (i + 1 < lines.length && !block.join('\n').includes(']')) {
          i += 1;
          block.push(lines[i]);
        }
      }

      out.push({ kind: 'kv', lines: block, key, value: rest });
      i += 1;
      continue;
    }

    out.push({ kind: 'other', lines: [line] });
    i += 1;
  }

  return out;
}

/**
 * Read a single-line basic string value.
 *
 * Returns both the decoded `text` (for the CSV) and the exact `raw` literal as
 * it appears in the source, including its quotes. The rewrite must substitute
 * `raw` verbatim: re-encoding the decoded text is not round-trip safe — a
 * source `\n` decodes to a real newline, which would re-encode as a real
 * newline and silently fail to match, leaving the TOML holding prose while the
 * CSV claims to own it.
 *
 * @returns {{raw: string, text: string}|null}
 */
function readString(value) {
  if (value == null) return null;
  const m = value.match(/^"((?:[^"\\]|\\.)*)"\s*(?:#.*)?$/);
  if (!m) return null;
  return {
    raw: `"${m[1]}"`,
    text: m[1]
      .replace(/\\"/g, '"')
      .replace(/\\n/g, '\n')
      .replace(/\\t/g, '\t')
      .replace(/\\\\/g, '\\'),
  };
}

/** Read a `"""..."""` value across the token's lines, honouring `\` continuations. */
function readMultiline(block) {
  const joined = block.join('\n');
  const start = joined.indexOf('"""');
  const end = joined.lastIndexOf('"""');
  if (start === -1 || end <= start) return null;
  return joined
    .slice(start + 3, end)
    .replace(/\r\n/g, '\n')      // normalise first: these files are mixed CRLF/LF
    .replace(/\\\n[ \t]*/g, '')  // TOML line-continuation backslashes
    .replace(/^\n/, '')          // TOML drops a newline straight after """
    .trim();
}

/** Read `["a", "b"]`, or null. */
function readArray(value) {
  if (value == null || !value.trimStart().startsWith('[')) return null;
  const inner = value.slice(value.indexOf('[') + 1, value.lastIndexOf(']'));
  const items = [...inner.matchAll(/"((?:[^"\\]|\\.)*)"/g)].map((m) => m[1]);
  return items;
}

/** Escape a value for a TOML basic string. */
function tomlEscape(s) {
  return s.replace(/\\/g, '\\\\').replace(/"/g, '\\"');
}

// ── Id generation ───────────────────────────────────────────────────────────

function slug(s) {
  return String(s)
    .toLowerCase()
    .replace(/['']/g, '')
    .replace(/[^a-z0-9]+/g, '_')
    .replace(/^_+|_+$/g, '')
    .slice(0, 40);
}

/**
 * Build a readable, stable id from the location of a value.
 * e.g. entity.alliance_battleship.system.power_reactor.display_name
 */
function makeId(prefix, stem, pathParts, key) {
  return [prefix, slug(stem), ...pathParts.map(slug).filter(Boolean), slug(key)]
    .filter(Boolean)
    .join('.');
}

// ── Extraction ──────────────────────────────────────────────────────────────

/** @type {Array<{id: string, context: string, en: string}>} */
const rows = [];
const seenIds = new Map();

function addRow(id, context, en) {
  // Ids must be unique. Two array entries with no discriminator can collide;
  // suffix rather than silently dropping one.
  let finalId = id;
  if (seenIds.has(finalId)) {
    const n = seenIds.get(finalId) + 1;
    seenIds.set(finalId, n);
    finalId = `${id}_${n}`;
  }
  seenIds.set(finalId, seenIds.get(finalId) ?? 1);
  rows.push({ id: finalId, context, en: `[${en}]` });
  return finalId;
}

/**
 * Track the current table path across header tokens, turning
 * `[[hull.system_hull]]` into path parts and folding in a discriminator.
 */
function makePathTracker(tokens) {
  // Pre-scan so a header can look ahead for its block's discriminator value.
  const discriminatorAt = new Map();
  for (let i = 0; i < tokens.length; i += 1) {
    if (tokens[i].kind !== 'header') continue;
    for (let j = i + 1; j < tokens.length && tokens[j].kind !== 'header'; j += 1) {
      const tk = tokens[j];
      if (tk.kind === 'kv' && DISCRIMINATORS.includes(tk.key)) {
        const v = readString(tk.value);
        if (v) { discriminatorAt.set(i, v.text); break; }
      }
    }
  }

  const counters = new Map();
  return function pathFor(index, header, isArray) {
    const parts = header.split('.').map((p) => p.replace(/^"|"$/g, ''));
    const disc = discriminatorAt.get(index);
    if (disc) return [...parts, disc];
    if (isArray) {
      const n = counters.get(header) ?? 0;
      counters.set(header, n + 1);
      return [...parts, String(n)];
    }
    return parts;
  };
}

/**
 * Extract and rewrite one TOML file.
 *
 * @param {string} file absolute path
 * @param {string} prefix id namespace ('entity' | 'world' | 'faction')
 * @param {Map<string,string>|null} nameMap entity-name → id, for reference rewriting
 * @param {boolean} collectOnly when true, only harvest `[[entity]] name` values
 * @returns {Promise<{src: string, names: Map<string,string>}>}
 */
async function processFile(file, prefix, nameMap, collectOnly) {
  const src = await readFile(file, 'utf8');
  const stem = path.basename(file, '.toml');
  const rel = path.relative(root, file).replace(/\\/g, '/');
  const tokens = tokenise(src);
  const pathFor = makePathTracker(tokens);
  const names = new Map();

  let current = [];
  let currentHeader = '';

  for (let i = 0; i < tokens.length; i += 1) {
    const tok = tokens[i];

    if (tok.kind === 'header') {
      current = pathFor(i, tok.header, tok.array);
      currentHeader = tok.header;
      continue;
    }
    if (tok.kind !== 'kv') continue;

    const where = currentHeader ? `[${currentHeader}] ${tok.key}` : `${tok.key} (top level)`;
    const context = `${rel} → ${where}`;

    // Reference keys: point at an entity name, never localised themselves.
    if (REFERENCE_KEYS.has(tok.key)) {
      if (collectOnly || !nameMap) continue;
      const single = readString(tok.value);
      if (single && nameMap.has(single.text)) {
        tok.lines[0] = tok.lines[0].replace(single.raw, `"${nameMap.get(single.text)}"`);
        continue;
      }
      const arr = readArray(tok.value);
      if (arr) {
        let line = tok.lines.join('\n');
        for (const item of arr) {
          if (nameMap.has(item)) {
            line = line.replace(`"${item}"`, `"${nameMap.get(item)}"`);
          }
        }
        tok.lines = line.split('\n');
      }
      continue;
    }

    if (!isLocalisable(tok.key, currentHeader, prefix)) continue;

    // Collect pass: only interested in world entity names.
    if (collectOnly) {
      if (tok.key === 'name' && currentHeader === 'entity') {
        const value = readString(tok.value)?.text;
        // Skip names already migrated on an earlier run. Without this the
        // collect pass mints an id *from an id* (world.entity.x.name →
        // world.entity.world_entity_x_name.name) and the reference-rewriting
        // pass then corrupts every `entity =` and `targets =` pointing at it —
        // while the definition itself, being guarded, stays correct. The result
        // is a world whose references silently resolve to nothing.
        if (value && !LOOKS_LIKE_ID.test(value)) {
          // Deliberately *not* namespaced by world file. Rust resolves entity
          // names through a single global name→uuid map, and the same name is
          // reused across worlds (raider_alpha appears in default.toml and
          // patrol.toml). One name must therefore mean one id everywhere, or
          // a cross-file `entity = "..."` reference would stop resolving.
          names.set(value, `world.entity.${slug(value)}.name`);
        }
      }
      continue;
    }

    if (tok.multiline) {
      const value = readMultiline(tok.lines);
      if (value == null || LOOKS_LIKE_ID.test(value)) continue;
      const id = addRow(makeId(prefix, stem, current, tok.key), context, value);
      const indent = tok.lines[0].match(/^\s*/)[0];
      tok.lines = [`${indent}${tok.key} = "${id}"`];
      continue;
    }

    const value = readString(tok.value);
    if (value == null) continue;
    // Already migrated on an earlier run — leave it alone. This is what makes
    // the script safely re-runnable when new prose is added to a TOML file.
    if (LOOKS_LIKE_ID.test(value.text)) continue;

    // `from` / `speaker` name the sender. When that sender is an entity in the
    // world, the value doubles as an entity reference — world::content matches
    // it against name_to_uuid to decide which ship a template belongs to. Point
    // it at the entity's id so the reference keeps resolving; the client then
    // renders the entity's own name as the sender label, which is what we want
    // anyway. A sender with no entity behind it ("Starcorp Command") falls
    // through and gets its own row.
    if ((tok.key === 'from' || tok.key === 'speaker') && nameMap && nameMap.has(value.text)) {
      tok.lines[0] = tok.lines[0].replace(value.raw, `"${nameMap.get(value.text)}"`);
      continue;
    }

    // A world entity name already has its id assigned by the collect pass.
    let id;
    if (tok.key === 'name' && currentHeader === 'entity' && nameMap && nameMap.has(value.text)) {
      id = nameMap.get(value.text);
      addRow(id, `${context} (entity display name; also the cross-reference key)`, value.text);
    } else {
      id = addRow(makeId(prefix, stem, current, tok.key), context, value.text);
    }

    const rewritten = tok.lines[0].replace(value.raw, `"${id}"`);
    if (rewritten === tok.lines[0]) {
      // Should be impossible now that we substitute the raw literal, but a
      // silent no-op here desyncs the CSV from the TOML, which is exactly the
      // failure mode that is hardest to notice later. Fail loudly instead.
      throw new Error(`${rel}: failed to rewrite ${tok.key} — raw literal not found in source line`);
    }
    tok.lines[0] = rewritten;
  }

  return { src: tokens.flatMap((t) => t.lines).join('\n'), names };
}

// ── CSV emission ────────────────────────────────────────────────────────────

function csvField(s) {
  return /[",\n\r]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
}

function toCsv(entries) {
  const lines = ['id,context,en'];
  for (const r of entries) {
    lines.push([r.id, r.context, r.en].map(csvField).join(','));
  }
  return `${lines.join('\n')}\n`;
}

// ── Main ────────────────────────────────────────────────────────────────────

async function tomlFiles(dir) {
  const abs = path.join(root, 'assets', dir);
  const names = await readdir(abs);
  return names.filter((n) => n.endsWith('.toml')).sort().map((n) => path.join(abs, n));
}

async function main() {
  const groups = [
    { dir: 'entities', prefix: 'entity' },
    { dir: 'worlds', prefix: 'world' },
    { dir: 'factions', prefix: 'faction' },
  ];

  // Pass 1 — harvest every world entity name so references can follow renames.
  const nameMap = new Map();
  for (const file of await tomlFiles('worlds')) {
    const { names } = await processFile(file, 'world', null, true);
    for (const [name, id] of names) {
      if (!nameMap.has(name)) nameMap.set(name, id);
    }
  }

  // Pass 2 — extract, rewrite, and write back.
  let files = 0;
  for (const { dir, prefix } of groups) {
    for (const file of await tomlFiles(dir)) {
      const before = await readFile(file, 'utf8');
      const { src } = await processFile(file, prefix, nameMap, false);
      if (src !== before) {
        files += 1;
        if (!DRY_RUN) await writeFile(file, src, 'utf8');
      }
    }
  }

  // Merge, never overwrite. Once a human has reviewed a line they remove its
  // square brackets, and possibly rewrite the text entirely — clobbering the
  // file on a re-run would silently throw that work away and re-bracket
  // approved copy. Existing rows win; only genuinely new ids are appended.
  const outDir = path.join(root, 'assets', 'strings');
  const outFile = path.join(outDir, 'strings.csv');

  let existing = [];
  try {
    const prior = parseCsv(await readFile(outFile, 'utf8'));
    const header = prior[0] ?? [];
    const idCol = header.indexOf('id');
    existing = prior.slice(1)
      .filter((r) => (r[idCol] || '').trim() !== '')
      .map((r) => ({ id: r[idCol], context: r[header.indexOf('context')] ?? '', en: r[header.indexOf('en')] ?? '' }));
  } catch { /* first run */ }

  const known = new Set(existing.map((r) => r.id));
  const added = rows.filter((r) => !known.has(r.id));
  const merged = [...existing, ...added];

  if (!DRY_RUN) {
    await mkdir(outDir, { recursive: true });
    await writeFile(outFile, toCsv(merged), 'utf8');
  }

  const prefix = DRY_RUN ? '[dry run] ' : '';
  console.log(`${prefix}${rows.length} strings found across ${files} rewritten TOML files`);
  console.log(`${prefix}${existing.length} existing rows kept, ${added.length} new rows added`);
  console.log(`${prefix}entity names remapped: ${nameMap.size}`);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
