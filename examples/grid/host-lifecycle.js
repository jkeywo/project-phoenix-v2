/** One page operation owns every host/join resource, including pending startup. */
export function createGridLifecycle() {
  let current = null;
  function stop() {
    const previous = current; current = null;
    previous?.dispose();
  }
  function begin() {
    stop();
    const cleanups = [];
    const operation = {
      current: () => current === operation,
      own(cleanup) { if (operation.current()) cleanups.push(cleanup); else cleanup(); },
      dispose() { for (const cleanup of cleanups.splice(0).reverse()) cleanup(); },
    };
    current = operation;
    return operation;
  }
  return { begin, stop };
}

/** Rust owns identity and protocol; this adapter owns only physical peers. */
export function createGridPeers(grid) {
  const peers = new Map();
  let closed = false;
  return {
    attach(peer) {
      if (closed) { peer.close(); return; }
      const handle = grid.open_peer(); peers.set(handle, peer);
      peer.on('close', () => { if (!closed && peers.delete(handle)) grid.close_peer(handle); });
      peer.on('data', raw => {
        if (closed || !peers.has(handle)) return;
        try {
          const reply = JSON.parse(grid.receive(handle, typeof raw === 'string' ? raw : JSON.stringify(raw)));
          if (reply.previous != null) {
            const old = peers.get(reply.previous);
            if (peers.delete(reply.previous)) grid.close_peer(reply.previous);
            old?.close();
          }
          if (reply.output) peer.send(JSON.stringify(reply.output), 'reliable');
        } catch { peers.delete(handle); grid.close_peer(handle); peer.close(); }
      });
    },
    publish(raw) { if (!closed) for (const handle of JSON.parse(grid.recipients())) peers.get(handle)?.send(raw, 'snapshot'); },
    close() {
      if (closed) return;
      closed = true;
      const owned = [...peers]; peers.clear();
      for (const [handle, peer] of owned) { grid.close_peer(handle); peer.close(); }
    },
  };
}
