/** editor/workshop-model-preview.js — the local render session behind the
 * Workshop Models preview panel (`gui/workshop-model-preview-panel.js`).
 *
 * Wraps `provider.modelPreview`'s capture/start/control/status/stop calls
 * behind the vocabulary the panel renders: `running`/`loading` govern which
 * controls are enabled, `stale` marks a picture whose source has moved on
 * since it was captured, and `error` carries the last refusal without ever
 * mutating the draft or selection it was asked to preview.
 */
export function createWorkshopModelPreview({ provider, mount, title, onChange }) {
  const backend = provider?.modelPreview || null;
  const available = Boolean(backend);
  let running = false, loading = false, stale = false, error = null, status = null, disposed = false;

  if (available) backend.mount(mount, { title });

  function notify() { if (!disposed && onChange) onChange(); }

  function snapshot() {
    return { available, running, loading, stale, error, status };
  }

  async function refresh(draftValue, selectionValue) {
    if (!available) return;
    loading = true; stale = false; error = null; notify();
    try {
      const captured = backend.capture(draftValue);
      const result = await backend.start(captured, selectionValue);
      status = result; running = true; loading = false; notify();
    } catch (thrown) {
      running = false; loading = false; error = thrown; notify();
      throw thrown;
    }
  }

  async function control(command) {
    if (!available || !running) return;
    try {
      const result = await backend.control(command);
      status = result; error = null; notify();
      return result;
    } catch (thrown) {
      error = thrown; notify();
      throw thrown;
    }
  }

  async function stop() {
    if (!running && !loading) return;
    running = false; loading = false;
    if (available) await backend.stop();
    notify();
  }

  function invalidate() {
    if (!running) return;
    stale = true; notify();
  }

  async function dispose() {
    if (disposed) return;
    disposed = true;
    if (running || loading) {
      running = false; loading = false;
      if (available) await backend.stop();
    }
  }

  return { snapshot, refresh, control, stop, invalidate, dispose };
}
