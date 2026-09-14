/** A one-use source bundle carried across a full page navigation. No live
 * simulation state, operator identity, credentials or draft history belongs here. */
const TTL = 5 * 60 * 1000;
const MAX_BYTES = 512 * 1024 * 1024;
const path = value => typeof value === 'string' && value.startsWith('assets/')
  && value.split('/').every(part => part && part !== '.' && part !== '..'
    && !/[\\:%?#\x00-\x1f\x7f-\x9f]/.test(part));

export function copyWorkshopSource({ selectedId, archives, base }) {
  if (typeof selectedId !== 'string' || !selectedId || !Array.isArray(archives)
      || !archives.length || archives.length > 128 || !base?.base_files || !base.base_asset_manifest) {
    throw Error('Invalid Workshop source bundle');
  }
  let total = 0;
  const ids = new Set();
  const copies = archives.map(entry => {
    if (typeof entry?.id !== 'string' || !entry.id || ids.has(entry.id)
        || !(entry.bytes instanceof Uint8Array) || !entry.bytes.length) throw Error('Invalid retained pack');
    ids.add(entry.id);
    total += entry.bytes.length;
    if (total > MAX_BYTES) throw Error('Workshop source bundle is too large');
    return { id: entry.id, bytes: Uint8Array.from(entry.bytes) };
  });
  if (!ids.has(selectedId)) throw Error('Selected source pack is unavailable');
  const base_files = Object.create(null);
  for (const [name, text] of Object.entries(base.base_files)) {
    if (!path(name) || !/\.(toml|rhai)$/.test(name) || typeof text !== 'string') throw Error('Invalid base source');
    total += new TextEncoder().encode(text).length;
    if (total > MAX_BYTES) throw Error('Workshop source bundle is too large');
    base_files[name] = text;
  }
  const base_asset_manifest = Object.create(null);
  for (const [name, entry] of Object.entries(base.base_asset_manifest)) {
    if (!path(name) || !Number.isSafeInteger(entry?.length) || entry.length < 1 || entry.length > MAX_BYTES
        || !Number.isInteger(entry.crc32) || entry.crc32 < 0 || entry.crc32 > 0xffffffff
        || !Array.isArray(entry.requires) || !entry.requires.every(path)) throw Error('Invalid base asset declaration');
    base_asset_manifest[name] = { length: entry.length, crc32: entry.crc32, requires: [...entry.requires] };
  }
  return { selectedId, archives: copies, base: { base_files, base_asset_manifest } };
}

export function createWorkshopHandoffStore({ indexedDB = globalThis.indexedDB,
  now = () => Date.now(), mint = () => globalThis.crypto.randomUUID() } = {}) {
  let database;
  async function open() {
    if (!indexedDB) throw Error('Workshop source storage is unavailable');
    if (!database) database = new Promise((resolve, reject) => {
      const request = indexedDB.open('phoenix-workshop-handoff', 1);
      request.onupgradeneeded = () => request.result.createObjectStore('source');
      request.onsuccess = () => {
        request.result.onversionchange = () => { request.result.close(); database = null; };
        resolve(request.result);
      };
      request.onerror = request.onblocked = () => reject(request.error || Error('Workshop source storage is unavailable'));
    }).catch(error => { database = null; throw error; });
    return database;
  }
  async function transact(operation) {
    const db = await open();
    return new Promise((resolve, reject) => {
      const transaction = db.transaction('source', 'readwrite');
      const store = transaction.objectStore('source');
      let result = null, failure;
      const request = store.get('pending');
      request.onsuccess = () => {
        try { result = operation(store, request.result); }
        catch (error) { failure = error; transaction.abort(); }
      };
      transaction.oncomplete = () => resolve(result);
      transaction.onabort = transaction.onerror = () => reject(failure || transaction.error || Error('Workshop source storage failed'));
    });
  }
  const current = record => record?.version === 1 && Number.isFinite(record.created)
    && now() >= record.created && now() - record.created < TTL;
  return {
    async save(source) {
      const payload = copyWorkshopSource(source);
      const token = mint();
      if (typeof token !== 'string' || !/^[a-zA-Z0-9-]{16,80}$/.test(token)) throw Error('Invalid source transfer token');
      return transact((store, previous) => {
        if (current(previous)) throw Error('Another source pack is waiting to open');
        store.put({ version: 1, created: now(), token, payload }, 'pending');
        return token;
      });
    },
    async take(token) {
      const payload = await transact((store, record) => {
        if (!current(record)) { if (record) store.delete('pending'); return null; }
        if (record.token !== token) return null;
        store.delete('pending');
        return record.payload;
      });
      return payload ? copyWorkshopSource(payload) : null;
    },
    clear: token => transact((store, record) => {
      if (record?.token === token) store.delete('pending');
    }),
  };
}
