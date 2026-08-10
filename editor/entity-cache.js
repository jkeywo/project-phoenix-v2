import { readFile, listDirectory, getProjectRoot } from './project-root.js';
import {
  resolveTemplate,
  canonicalTemplatePath,
  canonicalIncludePath,
  INCLUDES_KEY,
} from './entity-includes.js';

const cache = new Map();
/** path -> full resolution record { ok, config, provenance, isComposed, sources, error }. */
const resolutionCache = new Map();
const listeners = new Set();

/** Live cache of `path -> RESOLVED parsed TOML`. Exported for legacy callers
 * (entity-editor.js iterates and mutates this Map). For a composed hull this
 * holds the document with its include closure merged in, not the raw file. */
export const entityCache = cache;

/** The TOML parser the editor uses (browser global; overridable in tests). */
function parseToml(text) {
  return window.tomlParse(text);
}

/**
 * Read every template in a hull's include closure into an in-memory
 * `path -> text` map, following `includes` breadth-first. A path that cannot be
 * read or parsed is simply omitted — the pure resolver then reports it as the
 * appropriate located error (missing / parse), naming the declaring file.
 *
 * When `rootTextOverride` is supplied, the ROOT's text is taken from it instead
 * of disk (its fragments are still read fresh). The interactive save gate uses
 * this so a composed hull is resolved against its LIVE, unsaved authored text
 * rather than the stale on-disk root (issue #910).
 */
async function gatherClosure(rootPath, rootTextOverride = null) {
  const texts = new Map();
  const seen = new Set();
  const root = canonicalTemplatePath(rootPath);
  const queue = [root];
  while (queue.length > 0) {
    const p = queue.shift();
    if (seen.has(p)) continue;
    seen.add(p);
    let text;
    if (p === root && rootTextOverride != null) {
      text = rootTextOverride;
    } else {
      try {
        text = await readFile(p);
      } catch {
        continue; // missing — resolver surfaces it against the declaring file
      }
    }
    if (text == null) continue;
    texts.set(p, text);
    let value;
    try {
      value = parseToml(text);
    } catch {
      continue; // unparseable — resolver surfaces include-parse
    }
    const includes = value && Array.isArray(value[INCLUDES_KEY]) ? value[INCLUDES_KEY] : [];
    for (const ref of includes) {
      if (typeof ref !== 'string') continue;
      const child = canonicalIncludePath(p, ref);
      if (child && !seen.has(child)) queue.push(child);
    }
  }
  return texts;
}

/**
 * Resolve an entity template — including its `includes` closure — by
 * project-root-relative path. Returns the full resolution record:
 *
 *   { ok, config, provenance, isComposed, sources, error }
 *
 * `config` is the RESOLVED document (fragment fields merged in) that
 * preview/validation panes should read; `provenance` says which fragment
 * authored each field; `error` (an `IncludeError`) is set on a resolution
 * failure and names the declaring file rather than silently dropping the hull.
 *
 * A template that declares no `includes` takes a fast path identical to the old
 * loader: its parsed document is returned verbatim, uncomposed.
 */
export async function resolveEntityConfig(path) {
  // Reuse a prior SUCCESSFUL resolution only while its resolved config is still
  // in the public cache. An external `entityCache.clear()` / `.delete(path)`
  // (entity-editor.js mutates the Map directly; tests clear it) then forces a
  // fresh resolve rather than handing back a stale document.
  const prior = resolutionCache.get(path);
  if (prior && prior.ok && cache.has(path)) return prior;

  let rootText;
  try {
    rootText = await readFile(path);
  } catch (err) {
    // A root that cannot be read is not cached (it may appear later) and is not
    // a composition error — same policy the old loader had.
    return {
      ok: false,
      config: null,
      provenance: null,
      isComposed: false,
      sources: [],
      error: { category: 'read-error', message: err?.message || String(err), file: path, chain: [path] },
    };
  }

  let rootValue;
  try {
    rootValue = parseToml(rootText);
  } catch (err) {
    const res = {
      ok: false,
      config: null,
      provenance: null,
      isComposed: false,
      sources: [],
      error: { category: 'include-parse', message: `template is not valid TOML: ${err?.message || err}`, file: path, chain: [path] },
    };
    resolutionCache.set(path, res);
    console.warn(`[entity-cache] failed to parse ${path}: ${err?.message || err}`);
    return res;
  }

  // Fast path: an entity that declares no includes is uncomposed by definition
  // (nothing is pulled in). Behaviour is byte-for-byte the old loader's.
  if (!rootValue || !Object.prototype.hasOwnProperty.call(rootValue, INCLUDES_KEY)) {
    const res = {
      ok: true,
      config: rootValue,
      provenance: null,
      isComposed: false,
      sources: [canonicalTemplatePath(path)],
      error: null,
    };
    cache.set(path, rootValue);
    resolutionCache.set(path, res);
    return res;
  }

  // Composed: gather the closure and resolve it with the pure resolver.
  const texts = await gatherClosure(path);
  const result = resolveTemplate(path, (p) => (texts.has(p) ? texts.get(p) : null), parseToml);
  if (result.ok) {
    const res = {
      ok: true,
      config: result.resolved.value,
      provenance: result.resolved.provenance,
      isComposed: result.resolved.isComposed,
      sources: result.resolved.sources,
      error: null,
    };
    cache.set(path, result.resolved.value);
    resolutionCache.set(path, res);
    return res;
  }

  const res = {
    ok: false,
    config: null,
    provenance: null,
    isComposed: false,
    sources: [],
    error: result.error,
  };
  resolutionCache.set(path, res);
  console.warn(
    `[entity-cache] failed to resolve includes for ${path}: ${result.error.category}: ${result.error.message} [include chain: ${result.error.chain.join(' -> ')}]`,
  );
  return res;
}

/**
 * Resolve a hull's include closure using LIVE authored text as the root, for
 * the interactive save gate (issue #910). The save gate must validate the
 * RESOLVED document a composed hull will become at runtime, but it must NOT
 * trust the cache or the on-disk root — either can be stale against the edits
 * the user is about to save. So the root is seeded from `rootText` (the text
 * about to be written), while its fragments are read fresh from disk. Nothing
 * is cached: this is a throwaway resolution for validation only.
 *
 * @param {string} rootPath  Project-root-relative path of the hull being saved.
 * @param {string} rootText  The hull's LIVE authored TOML (with `includes`).
 * @returns {Promise<{ ok: true, value: object, isComposed: boolean }
 *                  | { ok: false, error: import('./entity-includes.js').IncludeError }>}
 */
export async function resolveEntityConfigFromText(rootPath, rootText) {
  const root = canonicalTemplatePath(rootPath);
  const texts = await gatherClosure(root, rootText);
  const result = resolveTemplate(root, (p) => (texts.has(p) ? texts.get(p) : null), parseToml);
  if (result.ok) {
    return { ok: true, value: result.resolved.value, isComposed: result.resolved.isComposed };
  }
  return { ok: false, error: result.error };
}

/** Load and parse an entity TOML by project-root-relative path.
 * Resolves the `includes` closure so composed hulls return their fully merged
 * document (issue #910). Returns the cached value if present, otherwise reads,
 * resolves, caches, returns. Returns null on read/parse/resolution failure —
 * the located error is retrievable via {@link getEntityResolution}. */
export async function loadEntityConfig(path) {
  if (cache.has(path)) return cache.get(path);
  const res = await resolveEntityConfig(path);
  return res.ok ? res.config : null;
}

/** Synchronous lookup of an already-cached RESOLVED entity config. */
export function getEntityConfig(path) {
  return cache.get(path) || null;
}

/** Synchronous lookup of the last resolution record (config + provenance +
 * error) for `path`, or null if it was never resolved. Lets validation/preview
 * surface a composed hull's provenance and any include failure. */
export function getEntityResolution(path) {
  return resolutionCache.get(path) || null;
}

/** Synchronous lookup of a resolved hull's provenance, or null. */
export function getEntityProvenance(path) {
  return resolutionCache.get(path)?.provenance || null;
}

/** Walk assets/entities/, load every *.toml into the cache.
 * No-ops (and returns []) if no project root is selected yet. */
export async function preloadEntityCache() {
  const root = await getProjectRoot().catch(() => null);
  if (!root) {
    console.warn('[entity-cache] no project root selected; entity palette will be empty until you pick one');
    return [];
  }

  let entries;
  try {
    entries = await listDirectory('assets/entities');
  } catch (err) {
    console.warn('[entity-cache] failed to list assets/entities:', err?.message || err);
    return [];
  }

  const results = [];
  for (const entry of entries) {
    if (entry.kind !== 'file') continue;
    if (!entry.name.endsWith('.toml')) continue;
    const path = `assets/entities/${entry.name}`;
    const config = await loadEntityConfig(path);
    if (config) {
      results.push({ ...entryToListItem(entry.name, config), config });
    }
  }
  return results;
}

/** Returns [{ name, path, tags }] for every entity in the cache. */
export function preloadEntityList() {
  const out = [];
  for (const [path, config] of cache) {
    out.push(entryToListItem(filenameFromPath(path), config, path));
  }
  return out;
}

/** Drop a single entry from the cache and notify listeners. */
export function invalidateEntity(path) {
  cache.delete(path);
  resolutionCache.delete(path);
  for (const cb of listeners) {
    try { cb(path); } catch (err) { console.error(err); }
  }
}

/** Drop everything and notify listeners (called with null). */
export function invalidateAll() {
  cache.clear();
  resolutionCache.clear();
  for (const cb of listeners) {
    try { cb(null); } catch (err) { console.error(err); }
  }
}

/** Subscribe to invalidations. Returns { unsubscribe }. */
export function onInvalidate(callback) {
  listeners.add(callback);
  return {
    unsubscribe: () => {
      listeners.delete(callback);
    },
  };
}

function filenameFromPath(path) {
  const tail = path.split('/').pop() || path;
  return tail.replace(/\.toml$/, '');
}

function entryToListItem(filename, config, path) {
  const name = filename.replace(/\.toml$/, '');
  return {
    name,
    path: path || `assets/entities/${filename}`,
    tags: (config && Array.isArray(config.tags)) ? config.tags : [],
  };
}
