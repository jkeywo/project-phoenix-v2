export function createWorkshopSourceTransfer({ store, read, operator, disconnect, navigate }) {
  let busy = false, disposed = false;
  return {
    async open(id) {
      if (busy || disposed) return false;
      const owner = operator()?.id;
      if (!owner) return false;
      busy = true;
      let token;
      try {
        const source = await read(id);
        if (disposed || operator()?.id !== owner) throw Error('GM connection changed');
        token = await store.save(source);
        if (disposed || operator()?.id !== owner) throw Error('GM connection changed');
        // Stop this peer's ordinary connection before an editable document
        // exists. Full navigation tears down its simulation and local UI.
        if (disconnect() !== true) throw Error('Could not leave the GM connection');
        // A fragment survives static hosts canonicalizing .html URLs without
        // retaining their query. The token never needs to reach an HTTP server.
        navigate(`workshop.html#source=${encodeURIComponent(token)}`);
        return true;
      } catch (error) {
        if (token) await store.clear(token).catch(() => {});
        throw error;
      } finally { busy = false; }
    },
    dispose() { disposed = true; },
  };
}

export function mountWorkshopSourceLink({ root, win, t }) {
  if (!root) return { refresh() {}, dispose() {} };
  const doc = root.ownerDocument;
  const panel = doc.createElement('section');
  panel.id = 'gm-workshop-source'; panel.hidden = true;
  const label = doc.createElement('label');
  label.htmlFor = 'gm-workshop-pack'; label.textContent = t('workshop.source_pack');
  const select = doc.createElement('select'); select.id = 'gm-workshop-pack';
  const button = doc.createElement('button'); button.type = 'button';
  button.id = 'gm-workshop-open'; button.textContent = t('workshop.leave_live');
  const status = doc.createElement('p'); status.setAttribute('role', 'status');
  panel.append(label, select, button, status);
  (doc.getElementById('gm-desk-brief') || root).append(panel);
  let handoffStore;
  // Storage can be denied (including a throwing indexedDB getter). Acquire it
  // only inside the guarded click operation, never while mounting GM controls.
  // Client/native GM bundles carry this shared presenter without the offline
  // editor modules. Load source transfer only for a deliberate supported open.
  const storage = async () => handoffStore ||= (await import('../editor/workshop-handoff.js'))
    .createWorkshopHandoffStore({ indexedDB: win.indexedDB });
  const transfer = createWorkshopSourceTransfer({
    store: { save: async source => (await storage()).save(source), clear: async token => (await storage()).clear(token) },
    read: id => win.__hostWorkshopSource(id), operator: () => win.__hostLocalGm?.(),
    disconnect: () => win.__hostWorkshopDisconnect(), navigate: url => win.location.assign(url),
  });
  let busy = false, signature = null;
  function refresh() {
    panel.hidden = typeof win.__hostWorkshopPacks !== 'function';
    if (panel.hidden || busy) return;
    const packs = win.__hostWorkshopPacks();
    const next = JSON.stringify(packs);
    if (signature !== next) {
      const selection = select.value;
      select.replaceChildren(...packs.map(pack => {
        const option = doc.createElement('option'); option.value = pack.id;
        option.textContent = pack.name || pack.id; return option;
      }));
      if (packs.some(pack => pack.id === selection)) select.value = selection;
      signature = next;
    }
    select.disabled = button.disabled = !packs.length || !win.__hostLocalGm?.();
    if (!packs.length) status.textContent = t('workshop.source_empty');
  }
  button.addEventListener('click', async () => {
    if (busy || button.disabled) return;
    busy = true; select.disabled = button.disabled = true;
    status.textContent = t('workshop.source_opening');
    try { if (!await transfer.open(select.value)) status.textContent = t('workshop.source_failed'); }
    catch { status.textContent = t('workshop.source_failed'); }
    finally { busy = false; refresh(); }
  });
  refresh();
  win.addEventListener('PhoenixWorkshopSourceReady', refresh);
  return { refresh, dispose() { win.removeEventListener('PhoenixWorkshopSourceReady', refresh); transfer.dispose(); panel.remove(); } };
}
