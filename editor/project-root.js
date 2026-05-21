let rootHandle = null;

const DB_NAME = 'phoenix-editor';
const STORE_NAME = 'project-root';
const DB_VERSION = 1;

const rootChangeListeners = new Set();

/**
 * Subscribe to project-root change events. The listener is fired AFTER
 * `pickProjectRoot()` has successfully persisted the new handle.
 * Returns `{ unsubscribe }`.
 */
export function onRootChanged(cb) {
  if (typeof cb !== 'function') return { unsubscribe: () => {} };
  rootChangeListeners.add(cb);
  return {
    unsubscribe: () => { rootChangeListeners.delete(cb); },
  };
}

function fireRootChanged(handle) {
  for (const cb of rootChangeListeners) {
    try { cb(handle); } catch (err) {
      // Swallow listener errors so one bad subscriber can't block save flow.
      console.warn('[project-root] onRootChanged listener threw:', err);
    }
  }
}

export function isSupported() {
  return typeof window !== 'undefined'
    && 'showDirectoryPicker' in window
    && 'FileSystemDirectoryHandle' in window;
}

export async function pickProjectRoot() {
  rootHandle = await window.showDirectoryPicker({ mode: 'readwrite' });
  await persistHandle(rootHandle);
  fireRootChanged(rootHandle);
  return rootHandle;
}

export async function getProjectRoot() {
  if (rootHandle) return rootHandle;
  rootHandle = await loadHandle();
  return rootHandle;
}

export async function readFile(relativePath) {
  const handle = await requireRoot();
  const parts = normalizePath(relativePath);
  let dir = handle;
  for (let i = 0; i < parts.length - 1; i++) {
    dir = await dir.getDirectoryHandle(parts[i]);
  }
  const fileHandle = await dir.getFileHandle(parts[parts.length - 1]);
  const file = await fileHandle.getFile();
  return await file.text();
}

export async function writeFile(relativePath, content) {
  const handle = await requireRoot();
  const parts = normalizePath(relativePath);
  let dir = handle;
  for (let i = 0; i < parts.length - 1; i++) {
    dir = await dir.getDirectoryHandle(parts[i], { create: true });
  }
  const fileHandle = await dir.getFileHandle(parts[parts.length - 1], { create: true });
  const writable = await fileHandle.createWritable();
  await writable.write(content);
  await writable.close();
}

/**
 * List entries in a directory relative to the project root.
 * Empty path means the root itself. Returns [{ name, kind }] where kind is
 * 'file' or 'directory'. Throws if no project root has been selected.
 */
export async function listDirectory(relativePath = '') {
  const handle = await requireRoot();
  const parts = normalizePath(relativePath);
  let dir = handle;
  for (let i = 0; i < parts.length; i++) {
    dir = await dir.getDirectoryHandle(parts[i]);
  }
  const results = [];
  if (typeof dir.entries === 'function') {
    for await (const [name, entry] of dir.entries()) {
      const kind = entry.kind
        || (typeof entry.getFile === 'function' ? 'file' : 'directory');
      results.push({ name, kind });
    }
  }
  return results;
}

/** For testing: inject a mock root handle */
export function _setRootHandleForTest(handle) {
  rootHandle = handle;
}

/** For testing: drop all root-change listeners. */
export function _resetListenersForTest() {
  rootChangeListeners.clear();
}

async function requireRoot() {
  const handle = await getProjectRoot();
  if (!handle) {
    throw new Error('No project root selected. Call pickProjectRoot() first.');
  }
  return handle;
}

function normalizePath(relativePath) {
  return relativePath.replace(/\\/g, '/').split('/').filter(Boolean);
}

function persistHandle(handle) {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(DB_NAME, DB_VERSION);
    request.onupgradeneeded = () => {
      request.result.createObjectStore(STORE_NAME);
    };
    request.onsuccess = () => {
      const tx = request.result.transaction(STORE_NAME, 'readwrite');
      const store = tx.objectStore(STORE_NAME);
      store.put(handle, 'root');
      tx.oncomplete = () => resolve();
      tx.onerror = () => reject(tx.error);
    };
    request.onerror = () => reject(request.error);
  });
}

function loadHandle() {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(DB_NAME, DB_VERSION);
    request.onupgradeneeded = () => {
      request.result.createObjectStore(STORE_NAME);
    };
    request.onsuccess = () => {
      const tx = request.result.transaction(STORE_NAME, 'readonly');
      const store = tx.objectStore(STORE_NAME);
      const get = store.get('root');
      get.onsuccess = () => resolve(get.result);
      get.onerror = () => reject(get.error);
    };
    request.onerror = () => reject(request.error);
  });
}
