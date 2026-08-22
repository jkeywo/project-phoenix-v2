/**
 * gui/strings.js — The string table. All user-facing text resolves through here.
 *
 * Text lives in assets/strings/strings.csv, one row per string:
 *
 *     id,context,en
 *     console.sensors.no_target,"Sensors — target name field, nothing locked","[NO TARGET]"
 *
 * `id` is the stable key code refers to, `context` tells a translator where the
 * string appears and what the placeholders mean, `en` is the English text.
 * Further locale columns (fr, de, ...) append to the right without code changes
 * — no code changes, but every row needs the new cell even when it is empty,
 * because scripts/check-strings.mjs holds every row to the header's field count.
 *
 * Square brackets around an `en` value mark it as agent-drafted placeholder copy
 * that no human has approved yet. They are stored literally in the CSV — this
 * module does not add or strip them. A human removes the brackets line by line
 * as they sign the copy off, so an unbracketed string on screen means "reviewed"
 * and a bracketed one means "still a first draft".
 *
 * This module is deliberately I/O-free so it unit-tests under vitest's `node`
 * environment. The fetch that populates it lives in gui/strings-boot.js.
 */

// ── CSV parsing ─────────────────────────────────────────────────────────────

/**
 * Parse RFC 4180 CSV into an array of string arrays.
 *
 * Hand-rolled rather than pulled from npm because the client ships as plain ES
 * modules with no bundler, and because we need exactly one feature beyond
 * `split(',')`: quoted fields. Those are not optional here — the comms dialogue
 * in assets/worlds/*.toml is multi-line and full of commas, so a naive split
 * silently shreds it.
 *
 * @param {string} text
 * @returns {string[][]} rows of fields; blank trailing lines dropped
 */
export function parseCsv(text) {
  const rows = [];
  let row = [];
  let field = '';
  let quoted = false;
  let i = 0;

  // Strip a UTF-8 BOM — Excel writes one, and it would otherwise become part
  // of the first column's header name.
  if (text.charCodeAt(0) === 0xfeff) i = 1;

  const endField = () => { row.push(field); field = ''; };
  const endRow = () => {
    endField();
    // A trailing newline produces one empty final row; drop it rather than
    // emitting a bogus entry with an empty id.
    if (!(row.length === 1 && row[0] === '')) rows.push(row);
    row = [];
  };

  while (i < text.length) {
    const ch = text[i];

    if (quoted) {
      if (ch === '"') {
        if (text[i + 1] === '"') { field += '"'; i += 2; continue; }
        quoted = false;
        i += 1;
        continue;
      }
      field += ch;
      i += 1;
      continue;
    }

    if (ch === '"' && field === '') { quoted = true; i += 1; continue; }
    if (ch === ',') { endField(); i += 1; continue; }
    if (ch === '\r') { i += 1; continue; }
    if (ch === '\n') { endRow(); i += 1; continue; }

    field += ch;
    i += 1;
  }

  // Final row without a trailing newline.
  if (field !== '' || row.length > 0) endRow();

  return rows;
}

/**
 * Build an id → text lookup from CSV text.
 *
 * @param {string} text raw strings.csv contents
 * @param {string} [locale='en'] column name to read
 * @returns {Map<string, string>}
 */
export function buildTable(text, locale = 'en') {
  const rows = parseCsv(text);
  if (rows.length === 0) return new Map();

  const header = rows[0];
  const idCol = header.indexOf('id');
  let textCol = header.indexOf(locale);

  if (idCol === -1) {
    throw new Error("strings.csv: missing required 'id' column");
  }
  if (textCol === -1) {
    // An unfinished translation column is not worth taking the UI down for.
    console.warn(`strings.csv: no '${locale}' column, falling back to 'en'`);
    textCol = header.indexOf('en');
    if (textCol === -1) throw new Error("strings.csv: missing required 'en' column");
  }

  const table = new Map();
  for (let r = 1; r < rows.length; r += 1) {
    const id = (rows[r][idCol] || '').trim();
    if (id === '') continue;
    if (table.has(id)) {
      console.warn(`strings.csv: duplicate id '${id}' — later row wins`);
    }
    table.set(id, rows[r][textCol] ?? '');
  }
  return table;
}

// ── Lookup ──────────────────────────────────────────────────────────────────

/** @type {Map<string, string>} */
let table = new Map();

/** Warn once per missing id — a re-rendering console would otherwise spam. */
const warned = new Set();

/**
 * Install a table. Called by gui/strings-boot.js at startup, and directly by
 * tests that want a fixture table.
 * @param {Map<string, string>} next
 */
export function setTable(next) {
  table = next;
  warned.clear();
}

/** @returns {Map<string, string>} the live table (read-only by convention) */
export function getTable() {
  return table;
}

/**
 * Resolve a string id, substituting `{placeholder}` tokens.
 *
 *     t('console.sensors.contacts', { n: 3 })  // "[3 CONTACTS]"
 *
 * A missing id renders as ⟨the.id⟩ rather than throwing or returning empty:
 * a visibly wrong console beats a blank one or a crashed render loop, and the
 * angle brackets are distinct enough from the placeholder square brackets to
 * read as "this is broken" rather than "this is unreviewed".
 *
 * @param {string} id
 * @param {Record<string, string|number>} [params]
 * @returns {string}
 */
export function t(id, params) {
  let text = table.get(id);

  if (text === undefined) {
    if (!warned.has(id)) {
      warned.add(id);
      console.warn(`strings: no entry for '${id}'`);
    }
    return `⟨${id}⟩`;
  }

  if (params) {
    text = text.replace(/\{(\w+)\}/g, (match, key) =>
      Object.prototype.hasOwnProperty.call(params, key) ? String(params[key]) : match,
    );
  }

  return text;
}

/** @returns {boolean} whether an id exists — for callers that want a fallback */
export function has(id) {
  return table.has(id);
}

/**
 * Resolve `value` as a string id when the table has it, else use `fallback`.
 *
 * This is the render-site half of the localisation rule: a field that MAY be a
 * string id (a hull class token, a rating name, a station's raw `name`) but is
 * just as often literal text — a mod pack's prose, a value the table never
 * covered. `has(value) ? t(value) : fallback` was being re-derived inline at
 * several render call sites with slightly different fallbacks (see
 * gui/manual-panel.js, gui/settings-panel.js, gui/components/ph-ship-picker.js,
 * client.html's stationLabel); this collects it in one place.
 *
 * This is deliberately distinct from `localiseTree`, `applyToDom` and the other
 * wire-boundary functions above: those resolve a whole payload once, at the
 * point it crosses in from the network or the DOM. `wireText` is for a single
 * value at the point it is about to be RENDERED, where the caller already knows
 * exactly what to show when there is no table entry.
 *
 * @param {string|null|undefined} value
 * @param {string} [fallback] shown when `value` is not a known id; defaults to
 *   `String(value)` (or '' for a nullish value) when omitted
 * @returns {string}
 */
export function wireText(value, fallback) {
  if (has(value)) return t(value);
  if (fallback !== undefined) return fallback;
  return value === null || value === undefined ? '' : String(value);
}

// ── Wire boundary ───────────────────────────────────────────────────────────

/**
 * Suffix pairing a parameter table to the text id it interpolates into. Mirrors
 * `TEXT_PARAMS_SUFFIX` in `src/core/messages.rs`; the two are one contract and
 * `tests/client/strings.test.js` pins them together.
 */
export const TEXT_PARAMS_SUFFIX = '_params';

/**
 * Resolve string ids anywhere in a decoded server message.
 *
 * The server is deliberately localisation-blind: entity names, system display
 * names, comms dialogue and objective text all live in TOML as string ids, and
 * Rust passes them through untouched. Rather than teach every console which of
 * its fields happen to be text, we resolve once here, at the point the message
 * is decoded.
 *
 * Only strings that are actually present in the table are substituted, so ids
 * are the sole thing that changes — uuids, system ids, tokens and numbers pass
 * through as-is. Ids only exist in the table because we minted them during
 * extraction, so a collision with a meaningful non-display value cannot happen.
 *
 * ## Parameterised ids
 *
 * A text field may be joined by a sibling field named `<field>_params` holding
 * an object of `{placeholder: value}`. When one is present, the id resolves as
 * `t(id, params)` instead of `t(id)`, so a runtime figure the server computed
 * lands inside the sentence rather than beside it on a panel. `ObjectiveSnapshot
 * .text_params` and `CommsMessage.body_params` are the wire fields that use it;
 * `TEXT_PARAMS_SUFFIX` in `src/core/messages.rs` is the Rust half of the
 * contract.
 *
 * Resolving by NAME rather than by a list of known fields is the same choice the
 * rest of this function makes: it walks a decoded message it knows nothing
 * about, so the sibling's name is the only thing that can tell it which table
 * belongs to which id. It also means the renderers need no change at all — text
 * arrives already interpolated, exactly as it always arrived already resolved.
 *
 * The params object is itself localised first, so a param whose value is a
 * string id (a system name, a faction) resolves before being substituted in.
 * A param the string does not mention is ignored, and a `{placeholder}` with no
 * param is left standing — both are `t`'s existing behaviour.
 *
 * Returns a new structure; the input is not mutated.
 *
 * @template T
 * @param {T} value
 * @returns {T}
 */
export function localiseTree(value) {
  if (typeof value === 'string') {
    return table.has(value) ? t(value) : value;
  }
  if (Array.isArray(value)) {
    return value.map(localiseTree);
  }
  if (value !== null && typeof value === 'object') {
    const out = {};
    for (const key of Object.keys(value)) {
      const params = paramsFor(value, key);
      if (params) {
        const id = value[key];
        out[key] = table.has(id) ? t(id, params) : id;
      } else {
        out[key] = localiseTree(value[key]);
      }
    }
    return out;
  }
  return value;
}

/**
 * The resolved parameter table for `obj[key]`, or `null` when there is not one.
 *
 * Guards on the *shape* rather than trusting the name: a field is only treated
 * as parameterised when it holds a string AND its `_params` sibling holds a
 * plain object. Anything else falls through to the ordinary walk, so a field
 * that merely happens to end in `_params` cannot change how its neighbour is
 * resolved.
 *
 * @param {Record<string, unknown>} obj
 * @param {string} key
 * @returns {Record<string, string|number>|null}
 */
function paramsFor(obj, key) {
  if (typeof obj[key] !== 'string') return null;
  const sibling = obj[`${key}${TEXT_PARAMS_SUFFIX}`];
  if (sibling === null || typeof sibling !== 'object' || Array.isArray(sibling)) return null;
  return localiseTree(sibling);
}

// ── DOM application ─────────────────────────────────────────────────────────

/**
 * Substitute text into a subtree.
 *
 *   <h2 data-i18n="console.sensors.scan_summary"></h2>
 *   <button data-i18n="console.sensors.cancel" data-i18n-attr="title:console.sensors.cancel_tip">
 *
 * `data-i18n` sets textContent; `data-i18n-attr` takes a comma-separated list of
 * `attribute:string.id` pairs for title/aria-label/placeholder and friends.
 *
 * Takes an explicit root so web components can pass their shadowRoot —
 * querySelectorAll does not cross the shadow boundary.
 *
 * @param {ParentNode} [root=document]
 */
export function applyToDom(root) {
  const scope = root || (typeof document !== 'undefined' ? document : null);
  if (!scope || typeof scope.querySelectorAll !== 'function') return;

  for (const el of scope.querySelectorAll('[data-i18n]')) {
    el.textContent = t(el.getAttribute('data-i18n'));
  }

  for (const el of scope.querySelectorAll('[data-i18n-attr]')) {
    for (const pair of el.getAttribute('data-i18n-attr').split(',')) {
      const sep = pair.indexOf(':');
      if (sep === -1) continue;
      const attr = pair.slice(0, sep).trim();
      const id = pair.slice(sep + 1).trim();
      if (attr === '' || id === '') continue;
      el.setAttribute(attr, t(id));
    }
  }
}
