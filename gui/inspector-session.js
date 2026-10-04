/** Shared Live Inspector navigation and identity-keyed reading storage.
 * Domain adapters decide which readings to remember and what missing or
 * terminal subjects mean. This module never interprets a reading. */
export function createInspectorSession(historyLimit) {
  let selected = null, cursor = -1;
  const history = [], readings = new Map();

  function move(offset) {
    const next = cursor + offset;
    if (next < 0 || next >= history.length) return false;
    cursor = next;
    selected = history[cursor];
    return true;
  }

  return {
    get selected() { return selected; },
    get canBack() { return cursor > 0; },
    get canForward() { return cursor >= 0 && cursor < history.length - 1; },
    /** Return an overflow identity no longer referenced by history or the
     * previous selection. World and Presentation discard that reading;
     * other adapters choose their own retention lifetime. */
    select(id, { record = true } = {}) {
      const previous = selected;
      let dropped = null;
      selected = id || null;
      if (record && selected && history[cursor] !== selected) {
        history.splice(cursor + 1);
        history.push(selected);
        if (history.length > historyLimit) dropped = history.shift();
        cursor = history.length - 1;
      }
      return dropped !== previous && !history.includes(dropped) ? dropped : null;
    },
    back: () => move(-1),
    forward: () => move(1),
    remember(id, reading) { readings.set(id, reading); },
    reading: id => readings.get(id),
    forget(id) { readings.delete(id); },
    /** Entity retains only visited identities still reachable by navigation. */
    pruneReadingsToHistory() {
      const reachable = new Set(history);
      reachable.add(selected);
      for (const id of readings.keys()) if (!reachable.has(id)) readings.delete(id);
    },
    reset() {
      selected = null; cursor = -1; history.length = 0; readings.clear();
    },
    state: () => ({ selected, history: [...history], cursor }),
  };
}

/** Bind once to a panel's existing controls. The adapter repaints its domain
 * after movement; focus follows that repaint and never a projection update. */
export function bindInspectorNavigation({ session, back, forward, status, render }) {
  const navigate = move => {
    if (!move()) return;
    render();
    status?.focus?.();
  };
  back?.addEventListener('click', () => navigate(session.back));
  forward?.addEventListener('click', () => navigate(session.forward));
  return () => {
    if (back) back.disabled = !session.canBack;
    if (forward) forward.disabled = !session.canForward;
  };
}
