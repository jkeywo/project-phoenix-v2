import { createDatabaseOpener } from './indexed-db.js';
/** One origin-local browser draft, stored as structured data in IndexedDB.
 * Independent of project-root's filesystem handles and private operator profile.
 * Writes serialize so an older edit cannot overwrite a newer snapshot. */
export function createWorkshopRecovery({ indexedDB = globalThis.indexedDB } = {}) {
  let queue = Promise.resolve();
  const open = createDatabaseOpener({
    indexedDB, name: 'phoenix-workshop-draft', store: 'draft',
    unavailable: 'Browser draft storage is unavailable.',
    failed: error => error || Error('Could not open browser draft storage.'),
    blocked: () => Error('Browser draft storage is blocked by another window.'),
  });
  async function transact(mode, operation) {
    const db = await open();
    return new Promise((resolve, reject) => {
      const transaction = db.transaction('draft', mode);
      let result;
      const request = operation(transaction.objectStore('draft'));
      request.onsuccess = () => { result = request.result; };
      transaction.oncomplete = () => resolve(result ?? null);
      transaction.onabort = transaction.onerror = () => reject(transaction.error || new Error('Browser draft storage failed.'));
    });
  }
  const enqueue = operation => {
    const pending = queue.then(operation);
    queue = pending.catch(() => {});
    return pending;
  };
  return {
    load: () => enqueue(() => transact('readonly', store => store.get('current'))),
    save: snapshot => enqueue(() => transact('readwrite', store => store.put(snapshot, 'current'))),
    clear: () => enqueue(() => transact('readwrite', store => store.delete('current'))),
  };
}
