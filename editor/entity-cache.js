import { readFile, listDirectory, getProjectRoot } from './project-root.js';

const cache = new Map();
const listeners = new Set();

/** Live cache of `path -> parsed TOML`. Exported for legacy callers
 * (entity-editor.js iterates and mutates this Map). */
export const entityCache = cache;

/** Load and parse an entity TOML by project-root-relative path.
 * Returns the cached value if present, otherwise reads, parses, caches, returns.
 * Returns null on read/parse failure. */
export async function loadEntityConfig(path) {
  if (cache.has(path)) return cache.get(path);
  try {
    const text = await readFile(path);
    const config = window.tomlParse(text);
    cache.set(path, config);
    return config;
  } catch (err) {
    console.warn(`[entity-cache] failed to load ${path}:`, err?.message || err);
    return null;
  }
}

/** Synchronous lookup of an already-cached entity config. */
export function getEntityConfig(path) {
  return cache.get(path) || null;
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
  for (const cb of listeners) {
    try { cb(path); } catch (err) { console.error(err); }
  }
}

/** Drop everything and notify listeners (called with null). */
export function invalidateAll() {
  cache.clear();
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
