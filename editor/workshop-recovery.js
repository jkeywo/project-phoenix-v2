/** One origin-local browser draft, stored as structured data in IndexedDB.
 * Independent of project-root's filesystem handles and private operator profile.
 * Writes serialize so an older edit cannot overwrite a newer snapshot. */
export function createWorkshopRecovery({ indexedDB = globalThis.indexedDB } = {}) {
  let database;
  let queue = Promise.resolve();
  async function open() {
    if (!indexedDB) throw new Error('Browser draft storage is unavailable.');
    if (!database) database = new Promise((resolve, reject) => {
      const request = indexedDB.open('phoenix-workshop-draft', 1);
      request.onupgradeneeded = () => request.result.createObjectStore('draft');
      request.onsuccess = () => {
        request.result.onversionchange = () => { request.result.close(); database = null; };
        resolve(request.result);
      };
      request.onerror = () => reject(request.error || new Error('Could not open browser draft storage.'));
      request.onblocked = () => reject(new Error('Browser draft storage is blocked by another window.'));
    }).catch(error => { database = null; throw error; });
    return database;
  }
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
