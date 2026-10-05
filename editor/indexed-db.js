/** Cache an IndexedDB connection, reopening after failure or version change. */
export function createDatabaseOpener({ indexedDB, name, store, unavailable, failed, blocked = failed }) {
  let database;
  return async function open() {
    if (!indexedDB) throw Error(unavailable);
    if (!database) database = new Promise((resolve, reject) => {
      const request = indexedDB.open(name, 1);
      request.onupgradeneeded = () => request.result.createObjectStore(store);
      request.onsuccess = () => {
        request.result.onversionchange = () => { request.result.close(); database = null; };
        resolve(request.result);
      };
      request.onerror = () => reject(failed(request.error));
      request.onblocked = () => reject(blocked(request.error));
    }).catch(error => { database = null; throw error; });
    return database;
  };
}
