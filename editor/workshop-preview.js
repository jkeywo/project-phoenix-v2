/** The Workshop's `provider.modelPreview` — the producer behind
 * `gui/workshop-model-preview-panel.js`.
 *
 * Deliberately the same machinery as the disposable Test: one iframe, one
 * private MessagePort, the same transferable snapshot guards, the same
 * dependency merger. What differs is which plugin the child boots. A second
 * capture path would be a second, quietly different definition of "captured",
 * and the isolation criterion is the whole point of this surface.
 */
import { createWorkshopTestPort, transferableTestSnapshot } from './workshop-test-frame.js';

/** A disposable preview owns one iframe and one port. Removing the frame
 * retires its entire WASM instance, which is what makes the preview disposable
 * rather than a long-lived renderer the Workshop has to keep correct. */
export function createWorkshopPreviewFrame({ mount, title, win = mount.ownerDocument.defaultView,
  // Trunk names the built page index.html whatever the source was called, so
  // the target directory is the address rather than the authored filename.
  url = new URL('../preview/index.html', import.meta.url).href }) {
  const frame = mount.ownerDocument.createElement('iframe');
  frame.className = 'workshop-model-preview-frame';
  frame.title = title;
  frame.setAttribute('sandbox', 'allow-scripts allow-same-origin');
  frame.setAttribute('referrerpolicy', 'no-referrer');
  const channel = new win.MessageChannel();
  let destroyed = false, settled = false, rejectLoad;
  const loaded = new Promise((resolve, reject) => {
    rejectLoad = reject;
    frame.addEventListener('load', () => { if (!settled) { settled = true; resolve(); } });
    frame.addEventListener('error', () => {
      if (!settled) { settled = true; reject(new Error('Workshop preview page failed to load')); }
    });
  });
  const loadTimeout = win.setTimeout(() => destroy(new Error('Workshop preview page did not load')), 30000);
  const connection = createWorkshopTestPort({ port: channel.port1, timer: win, failed: error => destroy(error) });
  function destroy(error = new Error('Workshop preview closed')) {
    if (destroyed) return;
    destroyed = true; win.clearTimeout(loadTimeout);
    if (!settled) { settled = true; rejectLoad(error); }
    connection.close(error); channel.port2.close(); frame.remove();
  }
  frame.src = url; mount.append(frame);
  return {
    async start(snapshot) {
      await loaded; win.clearTimeout(loadTimeout);
      if (destroyed) throw new Error('Workshop preview closed');
      // A missing preview build still fires `load` — on the 404 page — and then
      // answers nothing, so without this the panel waits out the whole start
      // timeout instead of saying what is wrong. The artifact is its own Trunk
      // target (see scripts/copy-workshop-preview.mjs), so its absence is a
      // build step that did not run, not a fault in the draft.
      if (!frame.contentWindow?.document?.getElementById('canvas')) {
        throw new Error('Workshop preview build is unavailable: run npm run build:workshop-preview');
      }
      frame.contentWindow.postMessage({ type: 'phoenix-workshop-preview-connect' }, new URL(url).origin, [channel.port2]);
      const captured = transferableTestSnapshot(snapshot);
      return connection.request({ operation: 'start', snapshot: captured.snapshot }, captured.transfer, 120000);
    },
    control(control) { return connection.request({ operation: 'control', control }); },
    status() { return connection.request({ operation: 'status' }); },
    destroy,
  };
}

/** Preparation and replacement are separate: a refused candidate never
 * destroys the picture the operator is already looking at. */
export function createBrowserWorkshopPreview({ prepare, frame, release = async () => {} }) {
  let current = null, generation = 0;
  const booting = new Set();
  const preparing = new Set();
  function cancelBoots() {
    for (const cancel of preparing) cancel(); preparing.clear();
    for (const run of booting) run.destroy(); booting.clear();
  }
  function retire(record) {
    record.run.destroy();
    void release(record.prepared).catch(() => {});
    if (current === record) current = null;
  }
  return {
    /** The draft, exactly as it stands. Identical to the Test's capture: the
     * same paths, the same bytes, the same refusal to read anything else. */
    capture: draft => Object.fromEntries(draft.paths()
      .map(path => [path, draft.isBinary(path) ? draft.bytes(path) : draft.read(path)])),
    async start(source, selection) {
      const accepted = ++generation;
      cancelBoots();
      let cancel;
      const cancelled = new Promise((_, reject) => {
        cancel = () => reject(new Error('Workshop preview start was cancelled'));
      });
      preparing.add(cancel);
      try {
        const prepared = await Promise.race([prepare(source, selection), cancelled]);
        if (accepted !== generation) {
          await release(prepared);
          throw new Error('Workshop preview start was cancelled');
        }
        const next = frame();
        booting.add(next);
        try {
          const status = await next.start(prepared);
          if (accepted !== generation) {
            await release(prepared);
            throw new Error('Workshop preview start was cancelled');
          }
          if (!status?.running) throw new Error(status?.error || 'Workshop preview could not start');
          booting.delete(next);
          const previous = current;
          current = { run: next, prepared };
          if (previous) retire(previous);
          return status;
        } catch (error) {
          if (booting.delete(next)) next.destroy();
          await release(prepared).catch(() => {});
          throw error;
        }
      } finally { preparing.delete(cancel); }
    },
    async control(control) {
      if (!current) return { running: false };
      const record = current, run = record.run;
      try {
        const status = await run.control(control);
        if (!status.running) retire(record);
        return status;
      } catch (error) { retire(record); throw error; }
    },
    async status() {
      if (!current) return { running: false };
      const record = current, run = record.run;
      try {
        const status = await run.status();
        if (!status.running) retire(record);
        return status;
      } catch (error) { retire(record); throw error; }
    },
    async stop() {
      generation++; cancelBoots();
      if (current) { const record = current; current = null; await release(record.prepared).catch(() => {}); record.run.destroy(); }
    },
  };
}
