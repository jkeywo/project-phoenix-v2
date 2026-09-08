/** Browser-host physical connections. Rust's BrowserConnections owns every
 * identity, replacement and recipient decision; this adapter owns objects,
 * event handlers and channel readiness. The phone never imports this module. */

function isOpen(conn) {
  return !!(conn && (conn.open || conn.readyState === 'open'));
}

/** Wait for exports, not PhoenixReady (which requires selecting a World and
 * constructing the ECS App). Never open crew admission with a missing owner. */
export async function hostConnectionRegistryReady(hostWindow) {
  if (!hostWindow.wasmBindings) {
    await new Promise(resolve => hostWindow.addEventListener(
      'TrunkApplicationStarted', resolve, { once: true },
    ));
  }
  return new hostWindow.wasmBindings.BrowserConnections();
}

export function createHostConnections(registry, {
  onMessage, onIdentified = () => {}, onDeparture = () => {},
}) {
  const connections = new Map(); // opaque incarnation handle -> physical link

  function closed(handle) {
    if (!connections.delete(handle)) return;
    const token = registry.close(handle);
    if (token != null) onDeparture(token);
  }

  function attach(conn) {
    const handle = registry.open();
    connections.set(handle, conn);
    conn.on('close', () => closed(handle));
    conn.on('data', raw => {
      let message, json;
      try {
        json = typeof raw === 'string' ? raw : JSON.stringify(raw);
        message = JSON.parse(json);
      } catch (_) { return; }
      if (!message || typeof message !== 'object') return;
      const before = registry.sender(handle);
      if (message.type === 'Identify') {
        const claimed = message.data && message.data.token;
        const result = JSON.parse(registry.bind(handle, typeof claimed === 'string' ? claimed : ''));
        if (!result.ok) {
          try { conn.send(JSON.stringify({ type: 'JoinRefused', data: { code: result.code } })); } catch (_) {}
          // Forget immediately even if physical close is asynchronous. A
          // refused identity cannot keep dispatching until the event arrives.
          closed(handle);
          conn.close();
          return;
        }
        // Ownership already changed in Rust, so even a synchronous close of
        // the old link cannot disconnect this Session. Delayed old callbacks
        // are harmless too, including peer-id reuse and snapshot transitions.
        if (result.previous != null) connections.get(result.previous)?.close();
        if (before == null) onIdentified(conn);
      }
      const token = registry.sender(handle);
      if (token != null) onMessage(token, json, handle);
    });
    return handle;
  }

  function targets(target, deliveryClass) {
    const handles = JSON.parse(registry.recipients(target));
    const out = [];
    for (const handle of handles) {
      const conn = connections.get(handle);
      const snapshot = conn?.snapshotChannel;
      if (deliveryClass === 'snapshot' && isOpen(snapshot)) out.push(snapshot);
      else if (isOpen(conn)) out.push(conn);
    }
    return out;
  }

  return { attach, targets, sender: handle => registry.sender(handle) };
}

if (typeof window !== 'undefined') {
  window.hostPeerRouting = { createHostConnections, hostConnectionRegistryReady };
}
