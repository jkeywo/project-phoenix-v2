/** Capture declared base dependencies once, then validate against those exact
 * bytes. Candidate and accepted-pack bytes are already in the source snapshot. */
import { crc32 } from './crc32.js';
import { assetDependencies } from './asset-dependencies.js';

const MAX_SNAPSHOT_BYTES = 512 * 1024 * 1024;

async function readDeclaredBytes(response, length) {
  if (!response.body?.getReader) {
    const bytes = new Uint8Array(await response.arrayBuffer());
    return bytes.length === length ? bytes : null;
  }
  const reader = response.body.getReader();
  const bytes = new Uint8Array(length);
  let offset = 0;
  try {
    for (;;) {
      const { value, done } = await reader.read();
      if (done) return offset === length ? bytes : null;
      if (value.byteLength > length - offset) {
        await reader.cancel();
        return null;
      }
      bytes.set(value, offset);
      offset += value.byteLength;
    }
  } finally { reader.releaseLock(); }
}

export function createWorkshopAssetSnapshot(source, {
  fetch = path => globalThis.fetch(new URL(`../${path}`, import.meta.url), { signal: AbortSignal.timeout(30000) }),
} = {}) {
  source = structuredClone(source);
  const typed = values => Object.fromEntries(Object.entries(values || {}).map(([path, bytes]) => {
    if (!(bytes instanceof Uint8Array) && (!Array.isArray(bytes)
        || bytes.some(value => !Number.isInteger(value) || value < 0 || value > 255))) {
      throw new Error(`Invalid immutable asset bytes: ${path}`);
    }
    return [path, Uint8Array.from(bytes)];
  }));
  source.base_assets = typed(source.base_assets);
  for (const pack of source.packs || []) if (pack.assets) pack.assets = typed(pack.assets);
  const assets = Object.assign(Object.create(null), source.base_assets || {});
  const suppliedBase = new Set(Object.keys(assets));
  const pending = new Map();
  const graph = Object.fromEntries(Object.entries(source.base_asset_manifest || {}).map(([path, value]) => [path, value.requires || []]));
  for (const pack of source.packs || []) {
    Object.assign(graph, pack.asset_dependencies || {});
    for (const [path, bytes] of Object.entries(pack.assets || {})) graph[path] = assetDependencies(path, bytes);
  }
  let retainedBytes = [source.base_assets, ...(source.packs || []).map(pack => pack.assets)]
    .flatMap(values => Object.values(values || {})).reduce((sum, bytes) => sum + bytes.length, 0);
  if (retainedBytes > MAX_SNAPSHOT_BYTES) throw new Error('Immutable asset snapshot is too large');
  let reservedBytes = 0;
  const supplied = path => Object.hasOwn(assets, path)
    || (source.packs || []).some(pack => Object.hasOwn(pack.assets || {}, path)
      || Object.hasOwn(pack.asset_dependencies || {}, path));
  const portable = path => typeof path === 'string' && path.startsWith('assets/')
    && path.split('/').every(part => part && part !== '.' && part !== '..' && !/[\\:%?#\x00-\x1f\x7f-\x9f]/.test(part));
  async function capture(path) {
    if (supplied(path) || !portable(path)) return;
    const descriptor = source.base_asset_manifest?.[path];
    // Only declared shipped content can fill a base dependency. A missing or
    // changed asset stays absent so the runtime names the missing dependency.
    if (!descriptor || !Number.isSafeInteger(descriptor.length) || descriptor.length < 1
        || descriptor.length > MAX_SNAPSHOT_BYTES || !Number.isInteger(descriptor.crc32)) return;
    if (!pending.has(path)) pending.set(path, (async () => {
      if (retainedBytes + reservedBytes + descriptor.length > MAX_SNAPSHOT_BYTES) return;
      reservedBytes += descriptor.length;
      try {
        const response = await fetch(path);
        if (!response.ok) return;
        const bytes = await readDeclaredBytes(response, descriptor.length);
        if (bytes && crc32(bytes) === descriptor.crc32) {
          assets[path] = bytes;
          retainedBytes += bytes.length;
        }
      } catch { /* The authoritative report supplies the missing path. */ }
      finally { reservedBytes -= descriptor.length; }
    })());
    await pending.get(path);
    if (!Object.hasOwn(assets, path)) pending.delete(path);
  }
  async function collect(paths) {
    const wanted = new Set(paths);
    // A buffer-only replacement must also validate every existing model
    // whose accessor ranges consume those bytes, using the same snapshot.
    const changedBuffers = new Set(paths.filter(path => path.endsWith('.bin')));
    const consumers = new Set();
    for (const [path, required] of Object.entries(graph)) {
      if (required.some(dependency => changedBuffers.has(dependency))) { wanted.add(path); consumers.add(path); }
    }
    for (const path of wanted) {
      for (const dependency of graph[path] || []) wanted.add(dependency);
    }
    // Bound simultaneous binary allocations; accepted bytes are retained for
    // the lifetime of this Authoring snapshot, independent of future delivery.
    for (const path of wanted) await capture(path);
    for (const path of consumers) {
      if (!supplied(path)) throw new Error(`Immutable model dependency is unavailable: ${path}`);
    }
    return wanted;
  }
  function snapshot(wanted, json = false) {
    const chosen = new Set([...suppliedBase, ...wanted]);
    const value = structuredClone({ ...source, base_assets: Object.fromEntries(Object.entries(assets)
      .filter(([path]) => chosen.has(path))) });
    if (json) {
      const arrays = values => Object.fromEntries(Object.entries(values || {}).map(([path, bytes]) => [path, Array.from(bytes)]));
      value.base_assets = arrays(value.base_assets);
      for (const pack of value.packs || []) if (pack.assets) pack.assets = arrays(pack.assets);
    }
    return value;
  }
  return {
    // Full Test capture must never enlarge a later validation call's JSON:
    // only its requested closure and original supplied dependencies cross it.
    async capture(paths) { return snapshot(await collect(paths), true); },
    async captureBuffers(paths) { return snapshot(await collect(paths)); },
    async captureAllBuffers() {
      const wanted = new Set([...Object.keys(source.base_asset_manifest || {}), ...Object.keys(assets),
        ...(source.packs || []).flatMap(pack => [...Object.keys(pack.assets || {}), ...Object.keys(pack.asset_dependencies || {})])]);
      const all = await collect([...wanted]);
      const resident = path => Object.hasOwn(assets, path)
        || (source.packs || []).some(pack => Object.hasOwn(pack.assets || {}, path));
      // Host import can use descriptor-only indexes because its admitted pack
      // owner already has the bytes. A fresh Test owns no such fallback.
      for (const path of all) {
        if (!resident(path)) throw new Error(`Immutable Test asset is unavailable: ${path}`);
      }
      return snapshot(all);
    },
  };
}
