/** Typed Console working state; operator preferences never enter this record. */
export function normalizeConsoleView(value) {
  const text = value => typeof value === 'string' && value ? value : null;
  const nav = value?.navigation;
  const navigation = nav && Number.isFinite(nav.zoom) && Number.isFinite(nav.center_x) && Number.isFinite(nav.center_y)
    ? { zoom: Math.max(0.25, Math.min(8, nav.zoom)), center_x: nav.center_x, center_y: nav.center_y,
      selected_uuid: text(nav.selected_uuid) } : null;
  return { inspected_record: text(value?.inspected_record), comms_thread: text(value?.comms_thread),
    overlay: text(value?.overlay), navigation,
    ...(['min', 'mid', 'detail'].includes(value?.energy_depth) ? { energy_depth: value.energy_depth } : {}) };
}

/** Repeated old snapshots must not undo a local gesture still travelling to the host. */
export function createConsoleViewSession({ send, apply }) {
  let current = normalizeConsoleView(null);
  let remoteSignature = null;
  let authority = null;
  let applying = false;
  let pending = [];
  return {
    get value() { return structuredClone(current); },
    change(patch) {
      if (applying) return false;
      const next = normalizeConsoleView({ ...current, ...patch });
      if (JSON.stringify(next) === JSON.stringify(current)) return false;
      current = next;
      pending.push(JSON.stringify(next));
      send(structuredClone(current));
      return true;
    },
    receive(value, host) {
      const next = normalizeConsoleView(value);
      const signature = JSON.stringify(next);
      if (signature === remoteSignature && host === authority) return false;
      const sameAuthority = host === authority;
      remoteSignature = signature;
      authority = host;
      const acknowledged = sameAuthority ? pending.indexOf(signature) : -1;
      if (acknowledged >= 0) {
        pending = pending.slice(acknowledged + 1);
        // A tick acknowledging an earlier gesture must not erase a later one.
        if (pending.length) return false;
      } else pending = [];
      current = next;
      applying = true;
      try { apply(structuredClone(current)); } finally { applying = false; }
      return true;
    },
  };
}
