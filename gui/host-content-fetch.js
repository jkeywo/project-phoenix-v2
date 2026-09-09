/** Shared pre-init and runtime world/script delivery (#1248). Rust owns
 * dependency discovery, overlay precedence, and terminal preload state. */
export function registerContentFetch(bindings, settled, fetcher = fetch) {
  bindings.set_world_fetch_callback(async (path, optional) => {
    const sidecar = /^assets\/models\/.+\.toml$/.test(path);
    try {
      const response = await fetcher(path, { signal: AbortSignal.timeout(30000) });
      if (!response.ok && !(optional && response.status === 404)) {
        throw new Error(`HTTP ${response.status}`);
      }
      const source = response.ok ? await response.text() : '';
      if (sidecar) bindings.wasm_push_sidecar_toml(path, source);
      else bindings.wasm_push_world_toml(path, source);
    } catch (error) {
      if (sidecar) bindings.wasm_push_sidecar_toml(path, '');
      else bindings.wasm_fail_world_fetch(path, String(error));
    }
    settled();
  });
}

if (typeof window !== 'undefined') window.registerContentFetch = registerContentFetch;
