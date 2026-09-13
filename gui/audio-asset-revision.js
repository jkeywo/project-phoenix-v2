/** Read-only revision discovery for pages served by the native delivery owner.
 * Host-local WASM revisions win. Other delivery sites get one bounded probe;
 * an HTML fallback or missing capability never starts a polling loop. */
export function createAudioAssetRevision({
  root = globalThis,
  fetch = (...args) => root.fetch(...args),
  interval = 1000,
  timeout = 2000,
} = {}) {
  let remote = 0, stopped = false, watching = false, advertised = false;
  let localTimer = null, requestTimer = null, deadline = null, controller = null;
  let callback = () => {}, previous = 0;
  const hasLocal = () => typeof root.__hostPackRevision === 'function';
  const read = () => hasLocal() ? `local:${root.__hostPackRevision()}` : remote;
  function changed() {
    const current = read();
    if (previous !== current) { previous = current; callback(); }
  }
  function cancelRequest() {
    if (requestTimer !== null) root.clearTimeout(requestTimer);
    if (deadline !== null) root.clearTimeout(deadline);
    requestTimer = deadline = null;
    controller?.abort();controller = null;
  }
  async function poll() {
    if (stopped || hasLocal()) return;
    const request = new AbortController();
    controller = request;
    deadline = root.setTimeout(() => request.abort(), timeout);
    let valid = false;
    try {
      const response = await fetch('/host/asset-revision.json', {
        cache: 'no-store', headers: { Accept: 'application/json' }, signal: request.signal,
      });
      if (stopped || hasLocal() || request.signal.aborted || !response.ok
          || response.headers?.get('content-type')?.split(';')[0].trim() !== 'application/json') return;
      const text = await response.text();
      if (stopped || hasLocal() || request.signal.aborted || text.length > 1024) return;
      const value = JSON.parse(text);
      if (value?.capability !== 'phoenix-native-asset-revision' || value.version !== 1
          || typeof value.revision !== 'string' || !/^(0|[1-9][0-9]{0,19})$/.test(value.revision)) return;
      advertised = valid = true;
      remote = `native:${value.revision}`;
      changed();
    } catch (_) { /* A failed delivery never creates a sound or changes authority. */ }
    finally {
      if (deadline !== null) root.clearTimeout(deadline);
      deadline = null;controller = null;
      if (!stopped && !hasLocal() && advertised)
        requestTimer = root.setTimeout(poll, valid ? interval : Math.max(interval, 5000));
    }
  }
  function watch(next) {
    if (watching) throw new Error('Audio asset revision already observed');
    watching = true;callback = next;previous = read();
    localTimer = root.setInterval(() => {
      if (hasLocal()) cancelRequest();
      changed();
    }, 100);
    localTimer?.unref?.();
    if (!hasLocal()) void poll();
    return () => {
      stopped = true;root.clearInterval(localTimer);cancelRequest();callback = () => {};
    };
  }
  return { read, watch };
}
