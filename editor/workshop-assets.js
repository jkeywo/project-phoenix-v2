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
  fetch = path => globalThis.fetch(new URL(`../${path}`, import.meta.url)),
} = {}) {
  source = structuredClone(source);
  const assets = Object.assign(Object.create(null), source.base_assets || {});
  const pending = new Map();
  const graph = Object.fromEntries(Object.entries(source.base_asset_manifest || {}).map(([path, value]) => [path, value.requires || []]));
  for (const pack of source.packs || []) {
    Object.assign(graph, pack.asset_dependencies || {});
    for (const [path, bytes] of Object.entries(pack.assets || {})) graph[path] = assetDependencies(path, bytes);
  }
  let retainedBytes = Object.values(assets).reduce((sum, bytes) => sum + bytes.length, 0);
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
          assets[path] = Array.from(bytes);
          retainedBytes += bytes.length;
        }
      } catch { /* The authoritative report supplies the missing path. */ }
      finally { reservedBytes -= descriptor.length; }
    })());
    await pending.get(path);
    if (!Object.hasOwn(assets, path)) pending.delete(path);
  }
  return {
    async capture(paths) {
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
      return structuredClone({ ...source, base_assets: assets });
    },
  };
}
