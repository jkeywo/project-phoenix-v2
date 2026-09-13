/** A disposable run belongs to one immutable draft revision. The provider
 * owns validation and process/iframe isolation; this shared controller owns
 * Authoring/Test transitions without ever patching a running simulation. */
export function createWorkshopTest({ provider, snapshot, onChange = () => {} }) {
  let generation = 0;
  let runGeneration = null;
  let mode = 'authoring';
  let run = null;
  let pending = 0;
  let polling = false;
  let disposed = false;
  let error = null;
  let queue = Promise.resolve();
  const state = () => ({ mode, run, busy: pending > 0, stale: run !== null && generation !== runGeneration, error });
  const notify = () => { if (!disposed) onChange(state()); };
  const serialize = action => {
    pending++; notify();
    const result = queue.then(action);
    queue = result.catch(() => {});
    return result.catch(reason => {
      if (!disposed) error = reason;
      throw reason;
    }).finally(() => { pending--; notify(); });
  };
  function accept(value) {
    if (!value || typeof value.running !== 'boolean') throw new Error('Invalid Workshop Test response');
    if (value.error) error = new Error(String(value.error));
    run = value.running ? value : null;
    if (!run) { mode = 'authoring'; runGeneration = null; }
    return value;
  }
  return {
    state,
    changed() { generation++; notify(); },
    start(selection) {
      if (disposed) return Promise.reject(new Error('Workshop closed'));
      // Capture before yielding or joining the operation queue. This includes
      // exact binary versions, and never a mutable document object.
      const captured = structuredClone(snapshot());
      const capturedGeneration = generation;
      const options = structuredClone(selection);
      return serialize(async () => {
        if (disposed) return;
        error = null;
        const result = await provider.start(captured, options);
        if (disposed) return;
        accept(result);
        if (run) { runGeneration = capturedGeneration; mode = 'test'; }
      });
    },
    authoring() {
      return serialize(async () => {
        if (disposed) return;
        error = null;
        if (run) {
          accept(await provider.control({ command: 'pause' }));
          if (run) accept(await provider.control({ command: 'visibility', visible: false }));
        }
        mode = 'authoring';
      });
    },
    test() {
      return serialize(async () => {
        if (disposed || !run) return;
        error = null;
        accept(await provider.control({ command: 'visibility', visible: true }));
        if (run) mode = 'test';
      });
    },
    control(command) {
      return serialize(async () => {
        if (disposed || !run || mode !== 'test') return;
        error = null;
        accept(await provider.control(command));
      });
    },
    poll() {
      if (disposed || pending || polling || !run) return Promise.resolve();
      polling = true;
      const result = queue.then(async () => {
        if (!disposed) accept(await provider.status());
      });
      queue = result.catch(() => {});
      return result.catch(reason => { if (!disposed) error = reason; throw reason; })
        .finally(() => { polling = false; notify(); });
    },
    stop() {
      return serialize(async () => {
        await provider.stop();
        if (!disposed) { run = null; mode = 'authoring'; runGeneration = null; error = null; }
      });
    },
    dispose() {
      if (disposed) return queue;
      disposed = true;
      // An accepted start may still be materializing when the document goes
      // away. Stop is queued after it, so no late child survives disposal.
      const stopped = queue.then(() => provider.stop());
      queue = stopped.catch(() => {});
      return stopped;
    },
  };
}
