/** One fresh iframe renders a captured Workshop draft through the shared
 * viewer. Content callbacks never fetch a missing path or consult mutable Live
 * state — the same rule the disposable Test runs under, for the same reason.
 *
 * The sibling of `editor/workshop-test-runtime.js`: that one boots the ordinary
 * simulation from explicit bytes, this one boots `ViewerPlugin`. Both take a
 * `{ files, selection, revision }` snapshot prepared by the same merger.
 */
export async function launchWorkshopPreview(snapshot, {
  bindings = async () => new Promise(resolve => {
    const finish = () => { if (globalThis.wasmBindings) { clearInterval(poll); resolve(globalThis.wasmBindings); } };
    // Trunk can dispatch TrunkApplicationStarted just before this listener is
    // installed, so poll as well rather than hanging on a one-shot race.
    const poll = setInterval(finish, 16);
    addEventListener('TrunkApplicationStarted', finish, { once: true });
    finish();
  }),
  timer = globalThis,
  signal,
} = {}) {
  const runtime = await bindings();
  signal?.throwIfAborted();
  const files = snapshot.files;
  const selection = snapshot.selection;
  let disposed = false;
  const waiters = new Map();

  // Rig sidecars and composed entity source are read through the ordinary
  // content cache rather than the asset server, so seed it from the capture.
  const read = path => (typeof files[path] === 'string' ? files[path] : '');
  // DEFERRED, never answered inside the callback. The runtime calls this from
  // Rust while it still holds its own content-cache borrow, so pushing straight
  // back in re-enters that borrow; the retained preview avoids it by answering
  // from a `fetch().then()`, and the disposable Test by queueing a promise.
  // An absent sidecar is delivered as the empty string, which is what "no rig"
  // means to the resolver — not an error, and not a reason to go looking on a
  // disk this page cannot reach.
  runtime.viewer_set_world_fetch_callback((path) => {
    queueMicrotask(() => { if (!disposed) runtime.viewer_push_sidecar_toml(path, read(path)); });
  });
  for (const [path, source] of Object.entries(files)) {
    if (typeof source === 'string' && /^assets\/models\/.+\.toml$/.test(path)) {
      runtime.viewer_push_sidecar_toml(path, source);
    }
  }

  function dispose() {
    disposed = true;
    for (const [id, resolve] of waiters) { timer.clearTimeout(id); resolve(); }
    waiters.clear();
    signal?.removeEventListener('abort', dispose);
  }
  signal?.addEventListener('abort', dispose, { once: true });

  /** The renderer publishes camelCase; the Inspector panel reads snake_case and
   * expects the whole reading under `stats`. Mapping here keeps the shared
   * ViewerPlugin JSON contract unchanged. */
  function status() {
    const text = runtime.viewer_stats();
    const measured = text ? JSON.parse(text) : null;
    if (!measured) return { running: !disposed, starting: true, stats: null };
    return {
      running: !disposed,
      starting: false,
      revision: snapshot.revision,
      selection,
      stats: {
        triangles: measured.triangles,
        meshes: measured.meshes,
        textures: measured.textures,
        measured_textures: measured.measuredTextures,
        texture_pixels: measured.texturePixels,
        largest_texture: measured.largestTexture,
        distance: measured.distance,
        extent: measured.extent,
        mode: measured.mode,
        level: measured.level,
        levels: measured.levels,
        settled: measured.settled,
        camera: measured.camera,
        // The viewer owns these as commands rather than measurements, so the
        // adapter reports what it last applied instead of inventing a reading.
        gizmos: applied.gizmos,
        lighting: applied.lighting,
      },
    };
  }

  // What the App actually booted with, so a reading never claims a setting the
  // renderer does not have: gizmos default on (PreviewSelection::gizmos_on) and
  // lighting is the viewer's own Mode::default(), which is Ambient.
  const applied = { gizmos: selection.gizmos !== false, lighting: 'ambient' };

  /** Wait for the App to say something, the way the disposable Test waits for
   * its first non-starting status.
   *
   * This is load-bearing rather than tidy. The session owner
   * (editor/workshop-model-preview.js) records `status` from what `start`
   * RESOLVES WITH and never polls again — a later reading only arrives when the
   * operator works a control. Those controls are disabled until the reading
   * says `settled`. So resolving before the viewer has measured anything leaves
   * the panel on "Loading…" with every control that could refresh it disabled:
   * a deadlock, not a slow frame. */
  async function settle(timeoutMs = 120000) {
    const began = Date.now();
    for (;;) {
      signal?.throwIfAborted();
      if (disposed) throw new Error('Workshop preview closed');
      const reading = status();
      if (reading.stats?.settled) return reading;
      if (Date.now() - began > timeoutMs) {
        throw new Error('The Workshop preview did not finish measuring the captured draft');
      }
      await new Promise(resolve => {
        const id = timer.setTimeout(() => { waiters.delete(id); resolve(); }, 32);
        waiters.set(id, resolve);
      });
    }
  }

  try {
    // `App::run()` on wasm throws to unwind the Rust stack while leaving the
    // winit loop alive. Called straight from an async module that throw becomes
    // a rejected promise and the loop never starts, so it goes on a plain task.
    await new Promise((resolve, reject) => timer.setTimeout(() => {
      try {
        runtime.viewer_workshop_preview_init(JSON.stringify(selection), files);
        resolve();
      } catch (error) {
        if (String(error?.message || error).includes('Using exceptions for control flow')) resolve();
        else reject(error);
      }
    }, 0));
    await settle();
  } catch (error) { dispose(); throw error; }

  return {
    status: async () => status(),
    async control(control) {
      switch (control.command) {
        case 'lod': runtime.viewer_set_lod_mode(control.mode, control.level ?? 0); break;
        case 'lighting': runtime.viewer_set_lighting(control.mode); applied.lighting = control.mode; break;
        case 'gizmos': runtime.viewer_set_gizmos(control.enabled); applied.gizmos = control.enabled; break;
        case 'distance': runtime.viewer_set_camera_distance(control.distance); break;
        case 'camera':
          runtime.viewer_set_camera(control.focus[0], control.focus[1], control.focus[2],
            control.radius, control.yaw, control.pitch);
          break;
        default: throw new Error('Invalid Workshop preview control');
      }
      return status();
    },
    dispose,
  };
}
