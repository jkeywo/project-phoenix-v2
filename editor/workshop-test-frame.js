/** A disposable browser Test owns one iframe and one private MessagePort.
 * No source root, operator profile, join identity or storage adapter crosses
 * this boundary. Removing the frame retires its entire WASM instance. */
export function createWorkshopTestPort({ port, timer = globalThis, failed = () => {} }) {
  let sequence = 0, closed = false;
  const pending = new Map();
  function close(reason = new Error('Workshop Test closed')) {
    if (closed) return;
    closed = true;
    port.removeEventListener('message', receive);
    port.removeEventListener('messageerror', broken);
    port.close();
    for (const request of pending.values()) { timer.clearTimeout(request.timeout); request.reject(reason); }
    pending.clear();
  }
  function broken() {
    const error = new Error('Workshop Test connection failed');
    close(error); failed(error);
  }
  function receive(event) {
    const value = event.data;
    const request = pending.get(value?.id);
    if (!request) return;
    pending.delete(value.id); timer.clearTimeout(request.timeout);
    if (value.error) {
      const error = new Error(String(value.error.message || value.error));
      if (value.error.report) error.report = value.error.report;
      request.reject(error);
    } else if (value.run && typeof value.run.running === 'boolean') request.resolve(value.run);
    else { request.reject(new Error('Invalid Workshop Test response')); broken(); }
  }
  port.addEventListener('message', receive);
  port.addEventListener('messageerror', broken);
  port.start();
  return {
    request(operation, transfer = [], timeoutMs = 5000) {
      if (closed) return Promise.reject(new Error('Workshop Test closed'));
      const id = ++sequence;
      return new Promise((resolve, reject) => {
        const timeout = timer.setTimeout(() => {
          const error = new Error('Workshop Test did not respond');
          close(error); failed(error);
        }, timeoutMs);
        pending.set(id, { resolve, reject, timeout });
        try { port.postMessage({ id, ...operation }, transfer); }
        catch (error) { pending.delete(id); timer.clearTimeout(timeout); reject(error); }
      });
    },
    close,
  };
}

/** Transfer defensive copies, never detach the document or dependency cache.
 * Text stays text; bytes never expand into a large JSON number array. */
export function transferableTestSnapshot(snapshot) {
  if (!snapshot || !snapshot.files || !snapshot.selection || typeof snapshot.revision !== 'string') {
    throw new Error('Invalid Workshop Test snapshot');
  }
  const files = Object.create(null), transfer = [];
  let length = 0;
  const entries = Object.entries(snapshot.files);
  if (entries.length > 16384) throw new Error('Workshop Test snapshot is too large');
  for (const [path, source] of entries) {
    if (!(path === 'scenarios.toml' || path.startsWith('assets/')) || /[:\\\x00-\x1f]/.test(path)
        || path.split('/').some(part => !part || part === '.' || part === '..')) {
      throw new Error('Invalid Workshop Test source path');
    }
    if (typeof source === 'string') { files[path] = source; length += new TextEncoder().encode(source).byteLength; }
    else if (source instanceof Uint8Array) {
      const bytes = source.slice(); files[path] = bytes; transfer.push(bytes.buffer); length += bytes.byteLength;
    } else throw new Error('Invalid Workshop Test source bytes');
    if (length > 512 * 1024 * 1024) throw new Error('Workshop Test snapshot is too large');
  }
  return { snapshot: { files, selection: structuredClone(snapshot.selection), revision: snapshot.revision,
    ...(snapshot.breakpoint ? { breakpoint: structuredClone(snapshot.breakpoint) } : {}) }, transfer };
}

export function createWorkshopTestFrame({ mount, title, win = mount.ownerDocument.defaultView,
  url = new URL('../workshop-test.html', import.meta.url).href }) {
  const frame = mount.ownerDocument.createElement('iframe');
  frame.className = 'workshop-test-viewscreen'; frame.title = title;
  frame.setAttribute('sandbox', 'allow-scripts allow-same-origin');
  frame.setAttribute('referrerpolicy', 'no-referrer');
  // Winit observes canvas intersection, so an unadopted frame below the
  // Authoring form cannot wait offscreen for its first simulation update.
  // Keep startup in the viewport without exposing controls or intercepting
  // input. Adoption returns the rendered view to its ordinary layout slot.
  frame.setAttribute('data-pending', '');
  frame.setAttribute('aria-hidden', 'true'); frame.inert = true;
  const channel = new win.MessageChannel();
  let destroyed = false, settled = false, rejectLoad;
  const loaded = new Promise((resolve, reject) => {
    rejectLoad = reject;
    frame.addEventListener('load', () => { if (!settled) { settled = true; resolve(); } });
    frame.addEventListener('error', () => { if (!settled) { settled = true; reject(new Error('Workshop Test page failed to load')); } });
  });
  const loadTimeout = win.setTimeout(() => destroy(new Error('Workshop Test page did not load')), 30000);
  const connection = createWorkshopTestPort({ port: channel.port1, timer: win, failed: error => destroy(error) });
  function destroy(error = new Error('Workshop Test closed')) {
    if (destroyed) return;
    destroyed = true; win.clearTimeout(loadTimeout);
    if (!settled) { settled = true; rejectLoad(error); }
    connection.close(error); channel.port2.close(); frame.remove();
  }
  frame.src = url; mount.append(frame);
  return {
    async start(snapshot) {
      await loaded; win.clearTimeout(loadTimeout);
      if (destroyed) throw new Error('Workshop Test closed');
      frame.contentWindow.postMessage({ type: 'phoenix-workshop-test-connect' }, new URL(url).origin, [channel.port2]);
      const captured = transferableTestSnapshot(snapshot);
      return connection.request({ operation: 'start', snapshot: captured.snapshot }, captured.transfer, 120000);
    },
    control(control) { return connection.request({ operation: 'control', control }); },
    status() { return connection.request({ operation: 'status' }); },
    visible(value) {
      frame.removeAttribute('data-pending');
      frame.style.visibility = value ? 'visible' : 'hidden'; frame.hidden = !value;
      frame.inert = !value; frame.setAttribute('aria-hidden', String(!value));
    },
    destroy,
  };
}

/** Preparation and replacement are separate: a refused candidate never
 * destroys the operator's previous run. All accepted runs use fresh frames. */
export function createBrowserWorkshopTest({ prepare, catalog, frame }) {
  let current = null, generation = 0;
  const booting = new Set();
  const preparing = new Set();
  function cancelBoots() {
    for (const cancel of preparing) cancel(); preparing.clear();
    for (const run of booting) run.destroy(); booting.clear();
  }
  function retire(run) { run.destroy(); if (current === run) current = null; }
  return {
    catalog,
    cancelStart() { generation++; cancelBoots(); },
    capture: draft => Object.fromEntries(draft.paths().map(path => [path, draft.isBinary(path) ? draft.bytes(path) : draft.read(path)])),
    async start(source, selection) {
      const accepted = ++generation;
      cancelBoots();
      let cancel;
      const cancelled = new Promise((_, reject) => { cancel = () => reject(new Error('Workshop Test start was cancelled')); });
      preparing.add(cancel);
      try {
        // Immutable dependency reads may still be in flight before any frame
        // exists. Cancellation must release the UI queue at that boundary too.
        const snapshot = await Promise.race([prepare(source, selection), cancelled]);
        if (accepted !== generation) throw new Error('Workshop Test start was cancelled');
        const next = frame();
        booting.add(next);
        try {
          const status = await next.start(snapshot);
          if (accepted !== generation) throw new Error('Workshop Test start was cancelled');
          if (!status?.running) throw new Error(status?.error || 'Workshop Test could not start');
          booting.delete(next); current?.destroy(); current = next; next.visible(true);
          return status;
        } catch (error) { if (booting.delete(next)) next.destroy(); throw error; }
      } finally { preparing.delete(cancel); }
    },
    async control(control) {
      if (!current) return { running: false };
      const run = current;
      try {
        // Browsers may suspend animation frames for display:none iframes. Show
        // the held run before waiting for its next clock acknowledgement.
        if (control.command === 'visibility' && control.visible) run.visible(true);
        const status = await run.control(control);
        if (!status.running) retire(run);
        else if (control.command === 'visibility') run.visible(control.visible);
        return status;
      } catch (error) { retire(run); throw error; }
    },
    async status() {
      if (!current) return { running: false };
      const run = current;
      try {
        const status = await run.status();
        if (!status.running) retire(run);
        return status;
      } catch (error) { retire(run); throw error; }
    },
    async stop() { generation++; cancelBoots(); current?.destroy(); current = null; },
  };
}
