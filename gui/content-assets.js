/** Read accepted pack bytes before the ordinary delivery URL. The host owns
 * acceptance and precedence; this adapter never installs or edits content. */
export async function fetchContentAsset(path, {
  read = globalThis.__hostReadPackAsset,
  fetch = (...args) => globalThis.fetch(...args),
} = {}) {
  const bytes = typeof path === 'string' && typeof read === 'function' ? read(path) : null;
  if (bytes instanceof Uint8Array) {
    const snapshot = Uint8Array.from(bytes);
    return { ok: true, arrayBuffer: async () => snapshot.buffer };
  }
  return fetch(path);
}
