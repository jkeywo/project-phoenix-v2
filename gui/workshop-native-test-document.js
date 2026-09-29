/** Consume only bounded, per-run loopback presentation resources. Native Test
 * owns the simulation; the existing GM workspace stays read-only here. */
export function mountNativeTestDocument({ win, image, hud, gmRoot, gm, status, unavailable,
  fetcher = (...args) => win.fetch(...args) }) {
  const LIMIT = 4 * 1024 * 1024;
  let identity = null, active = true, pending = false, sequence = -1;
  let controller = null, imageUrl = null, currentHud = null;
  const seenChannels = new Map(); let seenPresets = null;
  const showHud = () => { if (currentHud) hud.contentWindow?.__updateHud?.(currentHud); };
  hud.addEventListener('load', showHud);
  const route = (value, suffix) => {
    const url = new URL(value, win.location.href);
    if (url.origin !== win.location.origin || !new RegExp(`^/workshop-test-frame/[a-zA-Z0-9-]+/${suffix}$`).test(url.pathname)
      || url.search || url.hash) throw new Error('Invalid native Test presentation URL');
    return url;
  };
  async function read(url, signal) {
    const response = await fetcher(url.href, { cache: 'no-store', signal });
    if (!response.ok) throw new Error('Native Test presentation unavailable');
    const reader = response.body.getReader();
    const chunks = []; let size = 0;
    try {
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        size += value.byteLength;
        if (size > LIMIT) throw new Error('Native Test presentation exceeded its bound');
        chunks.push(value);
      }
    } finally { await reader.cancel().catch(() => {}); reader.releaseLock(); }
    const bytes = new Uint8Array(size); let offset = 0;
    for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.byteLength; }
    return bytes;
  }
  async function update(run) {
    if (!active || !run?.running) return;
    let frame, presentation;
    try {
      frame = route(run.frame_url, 'view\\.png');
      presentation = route(run.presentation_url, 'presentation\\.json');
      if (frame.pathname.slice(0, frame.pathname.lastIndexOf('/')) !== presentation.pathname.slice(0, presentation.pathname.lastIndexOf('/'))) return;
      if (identity && identity !== presentation.href) return; // A new run needs a new document.
      identity = presentation.href;
    } catch { status.textContent = unavailable; return; }
    const showingGm = run.view?.view === 'game-master';
    gmRoot.hidden = !showingGm;
    image.hidden = hud.hidden = showingGm;
    if (pending) return;
    pending = true;
    controller = new AbortController();
    try {
      const [pixels, encoded] = await Promise.all([read(frame, controller.signal), read(presentation, controller.signal)]);
      if (!active) return;
      const packet = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(encoded));
      if (!Number.isSafeInteger(packet.sequence) || packet.sequence < sequence || !packet.channels || typeof packet.channels !== 'object') return;
      sequence = packet.sequence;
      const next = win.URL.createObjectURL(new Blob([pixels], { type: 'image/png' }));
      image.src = next;
      if (imageUrl) win.URL.revokeObjectURL(imageUrl);
      imageUrl = next;
      for (const [name, payload] of Object.entries(packet.channels)) {
        if (typeof payload !== 'string' || seenChannels.get(name) === payload) continue;
        seenChannels.set(name, payload);
        if (name === 'hud') { currentHud = payload; showHud(); }
        else if (name.startsWith('gm_')) gm?.channel(name, payload);
      }
      if (typeof packet.role_presets === 'string' && packet.role_presets !== seenPresets) {
        gm?.setRolePresets(packet.role_presets); seenPresets = packet.role_presets;
      }
      status.textContent = '';
    } catch {
      controller?.abort();
      if (active) { image.removeAttribute('src'); status.textContent = unavailable; }
    } finally { pending = false; }
  }
  const receive = event => {
    if (event.source === win.parent && event.origin === win.location.origin && event.data?.type === 'phoenix-native-test-view') {
      void update(event.data.run);
    }
  };
  win.addEventListener('message', receive);
  return { update, dispose() {
    if (!active) return;
    active = false; controller?.abort();
    win.removeEventListener('message', receive); hud.removeEventListener('load', showHud);
    image.removeAttribute('src'); if (imageUrl) win.URL.revokeObjectURL(imageUrl);
    gm?.dispose();
  } };
}
