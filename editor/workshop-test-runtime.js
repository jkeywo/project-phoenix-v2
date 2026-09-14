/** One fresh iframe boots the ordinary browser simulation from explicit bytes.
 * Content callbacks never fetch a missing path or consult mutable Live state. */
export async function launchWorkshopTest(snapshot, {
  load = async () => {
    const module = await import(/* @vite-ignore */ new URL('../phoenix.js', import.meta.url).href);
    await module.default(); return module;
  },
  timer = globalThis,
  signal,
  onHud = () => {},
} = {}) {
  const runtime = await load();
  signal?.throwIfAborted();
  const files = snapshot.files;
  const selection = snapshot.selection;
  let failure = null, disposed = false, sequence = 0;
  const tasks = new Set();
  const requested = new Set();
  runtime.set_host_channel_callback?.((name, payload) => { if (!disposed && name === 'hud') onHud(payload); });
  const read = path => {
    const source = files[path];
    if (typeof source !== 'string') throw new Error(`Captured Test source is unavailable: ${path}`);
    return source;
  };
  const queued = callback => {
    const task = Promise.resolve().then(callback).catch(error => { failure ||= error; }).finally(() => tasks.delete(task));
    tasks.add(task);
  };
  runtime.set_config_request_callback(path => {
    if (requested.has(path)) return;
    requested.add(path);
    queued(() => {
      const rig = /^assets\/models\/.+\.toml$/.test(path);
      if (rig) return runtime.wasm_push_sidecar_toml(path, typeof files[path] === 'string' ? files[path] : '');
      try {
        const loader = path.startsWith('assets/complexity/') ? runtime.wasm_load_complexity : runtime.wasm_load_config;
        if (typeof loader !== 'function') throw new Error(`Runtime preload is unavailable: ${path}`);
        loader(path, read(path));
      }
      catch (error) { runtime.wasm_fail_preload_fetch(path, String(error)); throw error; }
    });
  });
  runtime.set_world_fetch_callback((path, optional) => queued(() => {
    const rig = /^assets\/models\/.+\.toml$/.test(path);
    if (rig) return runtime.wasm_push_sidecar_toml(path, typeof files[path] === 'string' ? files[path] : '');
    try { runtime.wasm_push_world_toml(path, read(path)); }
    catch (error) { runtime.wasm_fail_world_fetch(path, String(error)); if (!optional) throw error; }
  }));
  if (typeof files['assets/scenarios.toml'] === 'string') runtime.wasm_push_scenario_manifest(files['assets/scenarios.toml']);
  // Rhai's composed validation can request any captured child synchronously.
  // Populate its ordinary inbox before starting the root discovery pass.
  for (const [path, source] of Object.entries(files)) {
    if (typeof source === 'string' && /\.(toml|rhai)$/.test(path)) runtime.wasm_push_world_toml(path, source);
  }
  runtime.wasm_load_world(selection.world, read(selection.world), [selection.ship]);
  runtime.wasm_workshop_test_preload_ship(selection.ship, read(selection.ship));
  while (tasks.size) await Promise.all([...tasks]);
  signal?.throwIfAborted();
  if (failure) throw failure;
  const error = runtime.wasm_preload_error();
  if (error || !runtime.wasm_is_preload_complete()) throw new Error(error || 'Workshop Test preload is incomplete');
  runtime.wasm_select_ship(selection.ship);
  runtime.wasm_validate_stations(selection.ship, read(selection.ship));
  const waiters = new Map();
  async function waitFor(predicate, timeoutMs = 5000) {
    const began = Date.now();
    for (;;) {
      signal?.throwIfAborted();
      if (disposed) throw new Error('Workshop Test closed');
      if (failure) throw failure;
      const text = runtime.wasm_workshop_test_status();
      const status = text ? JSON.parse(text) : null;
      if (status && predicate(status)) return status;
      if (Date.now() - began > timeoutMs) throw new Error('Workshop Test runtime did not respond');
      await new Promise(resolve => { const id = timer.setTimeout(() => { waiters.delete(id); resolve(); }, 16); waiters.set(id, resolve); });
    }
  }
  function dispose() {
    disposed = true;
    for (const [id, resolve] of waiters) { timer.clearTimeout(id); resolve(); }
    waiters.clear(); signal?.removeEventListener('abort', dispose);
  }
  signal?.addEventListener('abort', dispose, { once: true });
  try {
    // Winit deliberately unwinds JS when handing control to the browser loop.
    // A genuine boot/renderer exception must still refuse this replacement.
    try { runtime.wasm_workshop_test_init(JSON.stringify({ selection, revision: snapshot.revision }), files); }
    catch (error) { if (!String(error?.message || error).includes('Using exceptions for control flow')) throw error; }
    await waitFor(status => !status.starting, 90000);
  } catch (error) { dispose(); throw error; }
  return {
    status: () => waitFor(() => true),
    async control(control) {
      const id = ++sequence;
      runtime.wasm_workshop_test_control(JSON.stringify({ id, control }));
      return waitFor(status => status.acknowledged >= id);
    },
    dispose,
  };
}
