/** The embedded Test page accepts one parent's private port exactly once.
 * Its adapter boots one captured snapshot; subsequent messages can only read
 * status or operate the finite clock vocabulary. */
/** The disposable Test's own control vocabulary. */
const TEST_CONTROLS = (value, exact) => {
  if (exact(value, ['command'])) return ['pause', 'resume', 'step', 'stop'].includes(value.command);
  return (exact(value, ['command', 'multiplier']) && value.command === 'rate' && [1, 2, 4, 8].includes(value.multiplier))
    || (exact(value, ['command', 'visible']) && value.command === 'visibility' && typeof value.visible === 'boolean');
};

export function attachWorkshopTestChild({ port, launch, controls = TEST_CONTROLS }) {
  let started = false, disposed = false, runtime = null, lastId = 0, pending = 0;
  const abort = new AbortController();
  let queue = Promise.resolve();
  const exact = (value, keys) => value && typeof value === 'object'
    && !Array.isArray(value) && Object.keys(value).length === keys.length && keys.every(key => Object.hasOwn(value, key));
  const validControl = value => controls(value, exact);
  function send(id, value) { if (!disposed) port.postMessage({ id, ...value }); }
  async function handle(value) {
    if (disposed) return;
    if (value.operation === 'start' && exact(value, ['id', 'operation', 'snapshot']) && !started) {
      started = true;
      runtime = await launch(value.snapshot, { signal: abort.signal });
      if (disposed) { runtime?.dispose?.(); runtime = null; return; }
      send(value.id, { run: await runtime.status() });
    } else if (value.operation === 'status' && exact(value, ['id', 'operation']) && runtime) {
      send(value.id, { run: await runtime.status() });
    } else if (value.operation === 'control' && exact(value, ['id', 'operation', 'control']) && runtime && validControl(value.control)) {
      send(value.id, { run: await runtime.control(value.control) });
    } else throw new Error('Invalid Workshop Test operation');
  }
  function receive(event) {
    const value = event.data;
    if (!Number.isSafeInteger(value?.id) || value.id <= lastId || pending >= 8) { dispose(); return; }
    lastId = value.id; pending++;
    queue = queue.then(() => handle(value)).catch(error => {
      send(value.id, { error: { message: String(error?.message || error), ...(error?.report ? { report: error.report } : {}) } });
    }).finally(() => { pending--; });
  }
  function dispose() {
    if (disposed) return;
    disposed = true; abort.abort(); port.removeEventListener('message', receive); port.removeEventListener('messageerror', dispose);
    port.close(); runtime?.dispose?.(); runtime = null;
  }
  port.addEventListener('message', receive); port.addEventListener('messageerror', dispose); port.start();
  return { dispose };
}

export function installWorkshopTestChild({ win = window, launch,
  type = 'phoenix-workshop-test-connect', controls = TEST_CONTROLS }) {
  let owner = null;
  function connect(event) {
    if (win.parent === win || event.source !== win.parent || event.origin !== win.location.origin
        || event.data?.type !== type || event.ports?.length !== 1) return;
    win.removeEventListener('message', connect);
    owner = attachWorkshopTestChild({ port: event.ports[0], launch, controls });
  }
  function dispose() { win.removeEventListener('message', connect); owner?.dispose(); }
  win.addEventListener('message', connect);
  win.addEventListener('pagehide', dispose, { once: true });
  return { dispose };
}
