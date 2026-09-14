/**
 * entity-includes.js — Composable entity-template resolution for the editor
 * (issue #910).
 *
 * A pure-JS twin of the Rust resolver in `src/entities/include_resolve.rs`
 * (plus the merge rules it delegates to in `src/entities/entity_override.rs`).
 * It turns one entity template plus its ordered `includes` into ONE resolved
 * document, and hands back PROVENANCE saying which fragment authored each field
 * and through which include chain — so the editor can render inherited fields
 * as inherited rather than guessing.
 *
 * This module is DOM-free and IO-free: it operates on already-read text through
 * a `read(path) -> string | null` source and a `parse(text) -> object` TOML
 * parser, exactly as the Rust `FragmentSource` trait is the sole seam through
 * which text enters the resolver. `entity-cache.js` (async fs) and
 * `mod-pack-export.js` (in-memory pack) supply those two callbacks.
 *
 * ── Merge semantics mirrored from the Rust `MergePolicy::ComposeFragments`
 *    layer (the ONLY policy this editor path uses) ─────────────────────────
 *   - depth-first, declared-order include resolution; the declaring template
 *     merges LAST so the includer always wins;
 *   - tables deep-merge; `tags` UNIONS; the keyed arrays in COMPOSE_KEYED_ARRAYS
 *     reconcile element-wise by their identity key (same key ⇒ deep-merge in
 *     place, new key ⇒ append, `{ id, _remove = true }` ⇒ drop the inherited
 *     entry); every other array REPLACES wholesale;
 *   - an explicit empty array is the only subtractive lever for a whole list;
 *   - the `_remove` tombstone and the `includes` key are authoring markers and
 *     never survive into the resolved document;
 *   - include paths are lexically canonicalised relative to the declaring file.
 *
 * Provenance reads the SAME identity table as the merge (see `recordLeaves`),
 * for the same reason the Rust code does: if the two disagreed, a merged-in
 * `[[system]]` array recorded as a wholesale leaf would prune every field an
 * earlier fragment contributed to it, and provenance would report a whole
 * system suite as authored by whichever fragment touched it last.
 */

/** The authored key listing a template's ordered includes. */
export const INCLUDES_KEY = 'includes';

/** The per-entry tombstone marker: `{ id = "x", _remove = true }`. */
export const REMOVE_KEY = '_remove';

/**
 * Arrays that reconcile by an identity key when composing fragments. Paths are
 * dotted and index-free — an element of `[[station]]` is reached at `station`,
 * so its own `[[station.rating]]` is reached at `station.rating`. Mirrors
 * `entity_override::COMPOSE_KEYED_ARRAYS` byte-for-byte; keep the two in sync.
 */
const COMPOSE_KEYED_ARRAYS = {
  'behaviour.doctrine': 'id',
  shield_arc: 'id',
  station: 'id',
  'station.rating': 'name',
  system: 'id',
  'torpedoes.tubes': 'id',
  'weapons_console.blaster_banks': 'id',
  'weapons_console.phaser_banks': 'id',
};

function isPlainObject(v) {
  return v !== null && typeof v === 'object' && !Array.isArray(v);
}

function deepClone(v) {
  if (typeof structuredClone === 'function') return structuredClone(v);
  return JSON.parse(JSON.stringify(v));
}

function deepEqual(a, b) {
  if (a === b) return true;
  if (Array.isArray(a) && Array.isArray(b)) {
    if (a.length !== b.length) return false;
    return a.every((x, i) => deepEqual(x, b[i]));
  }
  if (isPlainObject(a) && isPlainObject(b)) {
    const ka = Object.keys(a);
    const kb = Object.keys(b);
    if (ka.length !== kb.length) return false;
    return ka.every((k) => Object.prototype.hasOwnProperty.call(b, k) && deepEqual(a[k], b[k]));
  }
  return false;
}

/** What the merge does with the array at `path` (dotted, index-free). */
function arrayRule(path) {
  if (Object.prototype.hasOwnProperty.call(COMPOSE_KEYED_ARRAYS, path)) {
    return { kind: 'keyed', key: COMPOSE_KEYED_ARRAYS[path] };
  }
  if (path === 'tags') return { kind: 'union' };
  return { kind: 'replace' };
}

/** True for `{ … , _remove = true }`. */
export function isRemoval(entry) {
  return isPlainObject(entry) && entry[REMOVE_KEY] === true;
}

function joinPath(prefix, key) {
  return prefix === '' ? key : `${prefix}.${key}`;
}

// ── Merge (MergePolicy::ComposeFragments) ────────────────────────────────────

function mergeAt(path, template, override) {
  if (isPlainObject(template) && isPlainObject(override)) {
    const result = { ...template };
    for (const key of Object.keys(override)) {
      const child = joinPath(path, key);
      if (Object.prototype.hasOwnProperty.call(result, key)) {
        result[key] = mergeAt(child, result[key], override[key]);
      } else {
        result[key] = deepClone(override[key]);
      }
    }
    return result;
  }
  // An EMPTY override array never reconciles — it CLEARS (the only subtractive
  // lever for a whole list). Non-empty arrays follow the path's rule.
  if (Array.isArray(template) && Array.isArray(override) && override.length > 0) {
    const rule = arrayRule(path);
    if (rule.kind === 'keyed') return mergeKeyedArrayAt(path, template, override, rule.key);
    if (rule.kind === 'union') return unionArray(template, override);
    return deepClone(override);
  }
  return deepClone(override);
}

/** Set-union preserving template order, appending only what is new (`tags`). */
function unionArray(template, overrides) {
  const result = template.slice();
  for (const entry of overrides) {
    if (!result.some((e) => deepEqual(e, entry))) result.push(deepClone(entry));
  }
  return result;
}

/**
 * Reconcile two arrays whose elements carry `key`. Matched entries deep-merge
 * IN PLACE (so load-bearing order like `[[shield_arc]]` survives), new keys
 * append, and `_remove` drops the matched entry (and is never itself appended).
 */
function mergeKeyedArrayAt(path, template, overrides, key) {
  const result = template.map((e) => deepClone(e));
  for (const oEntry of overrides) {
    const id = isPlainObject(oEntry) && typeof oEntry[key] === 'string' ? oEntry[key] : null;
    if (id === null) {
      // Keyless entries have no identity to reconcile by — append.
      result.push(deepClone(oEntry));
      continue;
    }
    const pos = result.findIndex((e) => isPlainObject(e) && e[key] === id);
    const removal = isRemoval(oEntry);
    if (pos >= 0 && removal) {
      result.splice(pos, 1);
    } else if (pos < 0 && removal) {
      // A tombstone matching nothing is a no-op, not an error.
    } else if (pos >= 0 && isRemoval(result[pos])) {
      // Re-adding after a removal wins whole.
      result[pos] = deepClone(oEntry);
    } else if (pos >= 0) {
      result[pos] = mergeAt(path, result[pos], oEntry);
    } else {
      result.push(deepClone(oEntry));
    }
  }
  return result;
}

/** Drop every surviving `_remove` entry from a composed document. */
export function stripRemovals(value) {
  if (Array.isArray(value)) {
    return value.filter((e) => !isRemoval(e)).map(stripRemovals);
  }
  if (isPlainObject(value)) {
    const out = {};
    for (const [k, v] of Object.entries(value)) out[k] = stripRemovals(v);
    return out;
  }
  return value;
}

/**
 * Merge one fragment's contribution onto the accumulator under the compose
 * policy. `_remove` is honoured; the resulting document is stripped of any
 * tombstone the element-wise pass did not already consume.
 */
export function mergeComposeFragments(accumulator, value) {
  return stripRemovals(mergeAt('', accumulator, value));
}

// ── Provenance ───────────────────────────────────────────────────────────────

function isDescendant(key, prefix) {
  return (
    key.length > prefix.length &&
    key.startsWith(prefix) &&
    (key[prefix.length] === '.' || key[prefix.length] === '[')
  );
}

function joinField(prefix, key) {
  const quoted = /[.[\]= ]/.test(key) ? `"${key}"` : key;
  return prefix === '' ? quoted : `${prefix}.${quoted}`;
}

function insertLeaf(prefix, origin, out) {
  for (const k of [...out.keys()]) {
    if (isDescendant(k, prefix)) out.delete(k);
  }
  out.set(prefix, origin);
}

/**
 * Walk one fragment's contribution, recording who authored each leaf. `prefix`
 * is the PROVENANCE address (carries `[id=…]` element keys); `mergePath` is the
 * dotted, index-free path the identity table is keyed on. They must stay
 * separate — see the Rust `record_leaves` doc.
 */
function recordLeaves(prefix, mergePath, value, step, out) {
  const origin = () => ({ source: step.source, chain: step.chain.slice() });
  if (isPlainObject(value)) {
    const keys = Object.keys(value);
    if (keys.length === 0) {
      if (prefix !== '') insertLeaf(prefix, origin(), out);
      return;
    }
    out.delete(prefix);
    for (const key of keys) {
      if (key === REMOVE_KEY) continue;
      const path = joinField(prefix, key);
      const mergeChild = mergePath === '' ? key : `${mergePath}.${key}`;
      recordLeaves(path, mergeChild, value[key], step, out);
    }
    return;
  }
  if (Array.isArray(value)) {
    const rule = arrayRule(mergePath);
    if (rule.kind === 'keyed' && value.length > 0) {
      out.delete(prefix);
      value.forEach((element, index) => {
        const idVal =
          isPlainObject(element) && typeof element[rule.key] === 'string'
            ? element[rule.key]
            : null;
        const addressed = idVal !== null ? `${prefix}[${rule.key}=${idVal}]` : `${prefix}[${index}]`;
        if (isRemoval(element)) {
          for (const k of [...out.keys()]) {
            if (k === addressed || isDescendant(k, addressed)) out.delete(k);
          }
        } else {
          recordLeaves(addressed, mergePath, element, step, out);
        }
      });
      return;
    }
    insertLeaf(prefix, origin(), out);
    return;
  }
  insertLeaf(prefix, origin(), out);
}

/** Look up who authored `fieldPath` in a resolved document's provenance. */
export function fieldOrigin(provenance, fieldPath) {
  if (!provenance || !(provenance.fields instanceof Map)) return null;
  return provenance.fields.get(fieldPath) ?? null;
}

/** Canonical paths of every contributing template, in merge order. */
export function provenanceSources(provenance) {
  if (!provenance || !Array.isArray(provenance.order)) return [];
  return provenance.order.map((s) => s.source);
}

/**
 * True when `fieldPath` was authored by a template OTHER than the root hull —
 * i.e. it is inherited from a fragment. `rootPath` is the hull's own path.
 */
export function isFieldInherited(provenance, fieldPath, rootPath) {
  const origin = fieldOrigin(provenance, fieldPath);
  if (!origin) return false;
  return origin.source !== canonicalTemplatePath(rootPath);
}

/**
 * Classify a top-level section (e.g. `system`, `hull`) as authored by the hull,
 * inherited from a fragment, mixed, or absent — from provenance, not guesswork.
 * Returns `'authored' | 'inherited' | 'mixed' | 'none'`.
 */
export function sectionOrigin(provenance, section, rootPath) {
  if (!provenance || !(provenance.fields instanceof Map)) return 'none';
  const root = canonicalTemplatePath(rootPath);
  const contributors = new Set();
  for (const [path, origin] of provenance.fields) {
    if (path === section || isDescendant(path, section)) contributors.add(origin.source);
  }
  if (contributors.size === 0) return 'none';
  const hasRoot = contributors.has(root);
  const hasFragment = [...contributors].some((s) => s !== root);
  if (hasRoot && hasFragment) return 'mixed';
  return hasRoot ? 'authored' : 'inherited';
}

// ── Path canonicalisation ────────────────────────────────────────────────────

function normaliseSegments(path) {
  const leadingSlash = path.startsWith('/');
  const out = [];
  for (const segment of path.split('/')) {
    if (segment === '' || segment === '.') continue;
    if (segment === '..') {
      const last = out[out.length - 1];
      if (last !== undefined && last !== '..') out.pop();
      else out.push('..');
    } else {
      out.push(segment);
    }
  }
  const joined = out.join('/');
  return leadingSlash ? `/${joined}` : joined;
}

/** Canonicalise a template path: `\` → `/`, `.` dropped, `..` collapsed. */
export function canonicalTemplatePath(path) {
  return normaliseSegments(String(path).replace(/\\/g, '/'));
}

/**
 * Resolve an authored include reference against the template that declared it.
 * Returns `null` for a reference that is not resolvable relative to the
 * declarer: empty, root-absolute (`/…`), or drive-absolute (`C:\…`).
 */
export function canonicalIncludePath(declaringPath, include) {
  const ref = String(include).trim().replace(/\\/g, '/');
  if (ref === '' || ref.startsWith('/')) return null;
  if (ref.length >= 2 && ref[1] === ':') return null; // C:/… — absolute on Windows
  const declaring = String(declaringPath).replace(/\\/g, '/');
  const slash = declaring.lastIndexOf('/');
  const dir = slash >= 0 ? declaring.slice(0, slash) : '';
  const joined = dir === '' ? ref : `${dir}/${ref}`;
  return normaliseSegments(joined);
}

// ── Errors ───────────────────────────────────────────────────────────────────

/**
 * A composition failure carrying the include chain that reached it. Mirrors the
 * Rust `IncludeError`: `category` is a kebab-case slug (`include-cycle`,
 * `include-missing`, `include-parse`, `include-malformed`,
 * `include-invalid-template`), and `file`/`line`/`reference` name the DECLARING
 * template so the editor can point at the source, never a silent omission.
 */
export class IncludeError extends Error {
  constructor({ category, message, chain, file, line = null, reference = null }) {
    super(message);
    this.name = 'IncludeError';
    this.category = category;
    this.chain = chain;
    this.file = file;
    this.line = line;
    this.reference = reference;
  }

  chainDisplay() {
    return this.chain.join(' -> ');
  }
}

/** 1-based line of the first occurrence of `needle`, or null. Mirrors line_of. */
function lineOf(text, needle) {
  if (typeof text !== 'string' || !needle) return null;
  const idx = text.indexOf(needle);
  if (idx < 0) return null;
  let line = 1;
  for (let i = 0; i < idx; i++) if (text[i] === '\n') line += 1;
  return line;
}

// ── The resolver ─────────────────────────────────────────────────────────────

function takeIncludes(ctx, value, path, text) {
  const malformed = (message, reference) =>
    new IncludeError({
      category: 'include-malformed',
      message,
      chain: [...ctx.stack, path],
      file: path,
      line: lineOf(text, INCLUDES_KEY),
      reference,
    });

  if (!isPlainObject(value) || !Object.prototype.hasOwnProperty.call(value, INCLUDES_KEY)) {
    return [];
  }
  const raw = value[INCLUDES_KEY];
  delete value[INCLUDES_KEY];
  if (!Array.isArray(raw)) {
    throw malformed('`includes` must be an array of template paths', INCLUDES_KEY);
  }
  const out = [];
  for (const item of raw) {
    if (typeof item !== 'string') {
      throw malformed(
        '`includes` must be an array of template paths; found a non-string entry',
        INCLUDES_KEY,
      );
    }
    out.push(item);
  }
  return out;
}

function visit(ctx, path, decl, isRoot) {
  if (ctx.stack.includes(path)) {
    const chain = [...ctx.stack, path];
    throw new IncludeError({
      category: 'include-cycle',
      message: `include cycle: ${path} is already being resolved further up the chain (${chain.join(' -> ')})`,
      chain,
      file: decl ? decl.file : path,
      line: decl ? lineOf(decl.text, decl.reference) : null,
      reference: decl ? decl.reference : path,
    });
  }

  const text = ctx.read(path);
  if (text === null || text === undefined) {
    const chain = [...ctx.stack, path];
    throw new IncludeError({
      category: 'include-missing',
      message: `included template not found: ${path}`,
      chain,
      file: decl ? decl.file : path,
      line: decl ? lineOf(decl.text, decl.reference) : null,
      reference: decl ? decl.reference : path,
    });
  }

  let value;
  try {
    value = ctx.parse(text);
  } catch (e) {
    const chain = [...ctx.stack, path];
    throw new IncludeError({
      category: 'include-parse',
      message: `template is not valid TOML: ${e?.message || e}`,
      chain,
      file: path,
      line: null,
      reference: path,
    });
  }
  if (!isPlainObject(value)) {
    // A non-table root cannot be an entity template.
    const chain = [...ctx.stack, path];
    throw new IncludeError({
      category: 'include-parse',
      message: 'template root must be a table',
      chain,
      file: path,
      line: null,
      reference: path,
    });
  }

  if (isRoot) ctx.rootText = text;

  const includes = takeIncludes(ctx, value, path, text);

  ctx.stack.push(path);
  for (const reference of includes) {
    const child = canonicalIncludePath(path, reference);
    if (child === null) {
      const chain = [...ctx.stack, reference];
      throw new IncludeError({
        category: 'include-malformed',
        message: `include ${JSON.stringify(reference)} is not resolvable relative to ${path} — include paths are relative to the declaring template and must not be absolute`,
        chain,
        file: path,
        line: lineOf(text, reference),
        reference,
      });
    }
    visit(ctx, child, { file: path, text, reference }, false);
  }

  // The declaring template merges LAST, so the includer always wins.
  const step = { source: path, chain: [...ctx.stack] };
  ctx.accumulator =
    ctx.accumulator === null ? deepClone(value) : mergeComposeFragments(ctx.accumulator, value);
  recordLeaves('', '', value, step, ctx.provenance.fields);
  ctx.provenance.order.push(step);
  ctx.stack.pop();
}

function normaliseSource(source) {
  if (typeof source === 'function') return source;
  if (source && typeof source.read === 'function') return (p) => source.read(p);
  if (source && typeof source.get === 'function') return (p) => source.get(p) ?? null; // Map
  if (isPlainObject(source)) return (p) => (p in source ? source[p] : null);
  throw new TypeError('resolveTemplate: source must be a function, {read}, Map, or object');
}

/**
 * Resolve `rootPath` and its whole include closure into one document.
 *
 * @param {string} rootPath  Project-root-relative path of the declaring hull.
 * @param {Function|{read:Function}|Map|object} source  Serves raw TOML text by
 *   canonical path; returns `null`/`undefined` when it cannot serve a path.
 * @param {(text:string)=>object} parse  TOML parser (throws on invalid TOML).
 * @returns {{ ok: true, resolved: { path, value, provenance, isComposed, sources } }
 *          | { ok: false, error: IncludeError }}
 *
 * Every include must be readable; a fragment the source cannot serve is a
 * resolution error, as are cycles, unparseable fragments and malformed
 * `includes` declarations. Each error names the DECLARING file.
 */
export function resolveTemplate(rootPath, source, parse) {
  const read = normaliseSource(source);
  const root = canonicalTemplatePath(rootPath);
  const ctx = {
    read,
    parse,
    stack: [],
    accumulator: null,
    provenance: { order: [], fields: new Map() },
    rootText: null,
  };
  try {
    visit(ctx, root, null, true);
  } catch (error) {
    if (error instanceof IncludeError) return { ok: false, error };
    throw error;
  }
  const value = stripRemovals(ctx.accumulator === null ? {} : ctx.accumulator);
  const isComposed = ctx.provenance.order.length > 1;
  return {
    ok: true,
    resolved: {
      path: root,
      value,
      provenance: ctx.provenance,
      isComposed,
      sources: provenanceSources(ctx.provenance),
    },
  };
}

// ── Editing an inherited field: MATERIALISE-OVERRIDE (deliberate decision) ────
//
// DECISION (issue #910 AC): editing an inherited field MATERIALISES an override
// on the hull — it is NOT read-only.
//
// Why materialise rather than lock inherited fields read-only:
//   1. The editor exists to AUTHOR content. Read-only inherited fields would
//      make a composed hull only partly editable, which defeats the point of
//      giving hulls a fragment library in the first place.
//   2. It mirrors the runtime EXACTLY. The resolver's whole rule is
//      "the includer always wins": a value written on the hull's OWN document
//      beats the fragment's. Materialising a field = writing it onto the hull,
//      which is the same lever the resolver already honours. The next resolve
//      then reports that field's provenance as hull-authored — no special case.
//   3. It is reversible and non-destructive to composition. The hull keeps its
//      `includes`; only the one edited field is now hull-authored. Delete it
//      again and the field re-inherits. Crucially, the edit target stays the
//      hull's AUTHORED document (with `includes` intact) — preview/validation
//      read the RESOLVED document, but a SAVE writes the authored one, so
//      composition is never baked flat.
//
// The hull's other inherited fields remain inherited and keep tracking the
// fragment. Only the field the author touched is pinned onto the hull.

/**
 * Split a provenance field path into segments, honouring quoted keys and
 * `[key=val]` / `[index]` array addressing. `station[id=bridge].rating[name=x]`
 * → `[{name:'station', selector:{key:'id',val:'bridge'}}, {name:'rating', …}]`.
 */
function parseFieldPath(path) {
  const segments = [];
  let i = 0;
  const n = path.length;
  while (i < n) {
    // Read a (possibly quoted) key up to the next '.' or '['.
    let name = '';
    if (path[i] === '"') {
      i += 1;
      while (i < n && path[i] !== '"') {
        name += path[i];
        i += 1;
      }
      i += 1; // closing quote
    } else {
      while (i < n && path[i] !== '.' && path[i] !== '[') {
        name += path[i];
        i += 1;
      }
    }
    let selector = null;
    if (i < n && path[i] === '[') {
      let inner = '';
      i += 1;
      while (i < n && path[i] !== ']') {
        inner += path[i];
        i += 1;
      }
      i += 1; // closing bracket
      const eq = inner.indexOf('=');
      if (eq >= 0) selector = { key: inner.slice(0, eq), val: inner.slice(eq + 1) };
      else selector = { index: Number(inner) };
    }
    segments.push({ name, selector });
    if (i < n && path[i] === '.') i += 1;
  }
  return segments;
}

/**
 * Materialise an inherited field onto the hull's AUTHORED document: write
 * `value` at the provenance-style `fieldPath`, creating any intermediate tables
 * and keyed-array entries. Returns a NEW document (the input is not mutated), so
 * callers keep the pure-function discipline the editor uses.
 *
 * @param {object} authoredDoc  The hull's own parsed TOML (keeps `includes`).
 * @param {string} fieldPath    A provenance field path, e.g. `hull.hull_integrity`
 *                              or `system[id=helm-thrust].ai_only`.
 * @param {any} value           The value the author edited it to.
 * @returns {object} a clone of `authoredDoc` with the field materialised.
 */
export function materialiseOverride(authoredDoc, fieldPath, value) {
  const doc = isPlainObject(authoredDoc) ? deepClone(authoredDoc) : {};
  const segments = parseFieldPath(fieldPath);
  if (segments.length === 0) return doc;

  let cursor = doc;
  for (let s = 0; s < segments.length; s += 1) {
    const seg = segments[s];
    const isLast = s === segments.length - 1;
    if (seg.selector) {
      // Array segment: ensure an array under `seg.name`, then find/create the
      // addressed element.
      if (!Array.isArray(cursor[seg.name])) cursor[seg.name] = [];
      const arr = cursor[seg.name];
      let el;
      if (seg.selector.key !== undefined) {
        el = arr.find((e) => isPlainObject(e) && String(e[seg.selector.key]) === seg.selector.val);
        if (!el) {
          el = { [seg.selector.key]: seg.selector.val };
          arr.push(el);
        }
      } else {
        const idx = seg.selector.index;
        while (arr.length <= idx) arr.push({});
        if (!isPlainObject(arr[idx])) arr[idx] = {};
        el = arr[idx];
      }
      if (isLast) {
        // The path addressed the element itself — replace it wholesale.
        const pos = arr.indexOf(el);
        arr[pos] = deepClone(value);
      } else {
        cursor = el;
      }
    } else if (isLast) {
      cursor[seg.name] = deepClone(value);
    } else {
      if (!isPlainObject(cursor[seg.name])) cursor[seg.name] = {};
      cursor = cursor[seg.name];
    }
  }
  return doc;
}
