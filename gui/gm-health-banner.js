/**
 * gui/gm-health-banner.js — the reusable public-health status and banner
 * component (issue #1437, PRD #1419 story 10, presentation contract PRD #1418).
 *
 * Two things live here, and both are deliberately host-agnostic so the M5 live
 * restore surfaces (#1446/#1447) can reuse them without a second copy:
 *
 *  - `parseGmHealthProjection` — the strict reader for the `gm_health` Host
 *    Channel payload. Same posture as every other GM DTO adapter: a malformed
 *    row is dropped rather than rendered as `undefined`, and a payload that is
 *    not the expected shape at all is rejected whole so the previous honest
 *    picture stays on screen.
 *  - `createGmHealthBanner` — draws technical failure rows into ANY container.
 *    The GM desk points it at `#gm-attention-banners`, the region issue #1433
 *    deliberately placed outside the attention queue's filter, snooze and hold
 *    path. A live-restore surface can point the same component somewhere else
 *    and get identical wording, identical state words and identical actions.
 *
 * # Why the state is a WORD
 *
 * PRD #1418 story 27: status must not depend on colour. Every row here carries
 * its state as a translated noun beside the sentence — "Disconnected",
 * "Restoring", "Behind", "Paused" — and only decorates it with colour and a
 * `data-state` attribute afterwards. Forced colours take the decoration and
 * leave the meaning.
 *
 * # Why banners are reconciled rather than rebuilt
 *
 * A banner carries a button. Replacing the whole region on every projection —
 * which arrives whenever any peer's watermark moves — would throw that button
 * away under a keyboard operator mid-press, which is exactly the moving-target
 * failure PRD #1418 story 23 names. Rows are therefore matched by id and only
 * rewritten when their own content changed.
 *
 * Nothing in this file submits a command. A banner's action is a navigation on
 * the desk that already exists.
 */

/** Every state the Rust projection may report, least to most severe. */
export const GM_HEALTH_STATES = Object.freeze([
  'live', 'paused', 'stale', 'recovering', 'disconnected',
]);

/** Every alert kind this build knows how to draw. Unknown kinds still render —
 * the reason id carries the meaning — so a newer host is never silent. */
export const GM_HEALTH_ALERT_KINDS = Object.freeze([
  'station_disconnected', 'ship_peer_lost', 'operator_disconnected',
  'recovery_in_progress', 'recovery_failed',
  // The live-restore states (issue #1446), reported through this same banner
  // rather than a second restore-only surface.
  'live_restore_in_progress', 'live_restore_settled',
]);

/** The String Table id naming one state. */
export function healthStateLabelId(state) {
  return `server.gm.health.state.${GM_HEALTH_STATES.includes(state) ? state : 'live'}`;
}

/** Severity rank, for "what is the worst thing on this desk right now". */
export function healthStateRank(state) {
  const at = GM_HEALTH_STATES.indexOf(state);
  return at < 0 ? 0 : at;
}

function reference(value) {
  return value && typeof value.entity_id === 'string' && value.entity_id.length > 0
    ? { entity_id: value.entity_id, name: typeof value.name === 'string' ? value.name : value.entity_id }
    : null;
}

function reason(value) {
  if (!value || typeof value.id !== 'string' || value.id.length === 0) return null;
  return {
    id: value.id,
    params: value.params && typeof value.params === 'object' ? { ...value.params } : {},
  };
}

/**
 * Strictly validate one `gm_health` payload.
 *
 * @returns the projection, or `null` when the payload is not one.
 */
export function parseGmHealthProjection(payload) {
  let value = payload;
  if (typeof value === 'string') {
    try { value = JSON.parse(value); } catch (_) { return null; }
  }
  if (!value || typeof value !== 'object') return null;
  if (!Array.isArray(value.peers) || !Array.isArray(value.alerts)) return null;
  const tick = Number.isSafeInteger(value.tick) && value.tick >= 0 ? value.tick : 0;
  const state = (candidate) => (GM_HEALTH_STATES.includes(candidate) ? candidate : null);

  const peers = [];
  const seenPeers = new Set();
  for (const row of value.peers) {
    if (!row || typeof row.id !== 'string' || row.id.length === 0 || seenPeers.has(row.id)) continue;
    const rowState = state(row.state);
    if (!rowState) continue;
    seenPeers.add(row.id);
    peers.push({
      id: row.id,
      ship: reference(row.ship),
      operators: Array.isArray(row.operators) ? row.operators.filter((id) => typeof id === 'string') : [],
      state: rowState,
      behind_ticks: Number.isSafeInteger(row.behind_ticks) && row.behind_ticks >= 0 ? row.behind_ticks : null,
      local: row.local === true,
    });
  }

  const stations = [];
  // The host already resolves one row per Station seat (see src/gm_health.rs);
  // this only guards against a malformed feed repeating an id.
  const seenStations = new Set();
  for (const row of Array.isArray(value.stations) ? value.stations : []) {
    if (!row || typeof row.id !== 'string' || row.id.length === 0 || seenStations.has(row.id)) continue;
    const rowState = state(row.state);
    if (!rowState || typeof row.name !== 'string') continue;
    seenStations.add(row.id);
    stations.push({
      id: row.id,
      station_id: typeof row.station_id === 'string' ? row.station_id : row.id,
      name: row.name,
      ship: reference(row.ship),
      operator: typeof row.operator === 'string' ? row.operator : '',
      state: rowState,
    });
  }

  const operators = [];
  const seenOperators = new Set();
  for (const row of Array.isArray(value.operators) ? value.operators : []) {
    if (!row || typeof row.id !== 'string' || row.id.length === 0 || seenOperators.has(row.id)) continue;
    const rowState = state(row.state);
    if (!rowState) continue;
    seenOperators.add(row.id);
    operators.push({
      id: row.id,
      name: typeof row.name === 'string' ? row.name : '',
      state: rowState,
    });
  }

  const alerts = [];
  const seenAlerts = new Set();
  for (const row of value.alerts) {
    if (!row || typeof row.id !== 'string' || row.id.length === 0 || seenAlerts.has(row.id)) continue;
    const rowReason = reason(row.reason);
    const severity = state(row.severity);
    if (!rowReason || !severity || typeof row.kind !== 'string') continue;
    seenAlerts.add(row.id);
    alerts.push({
      id: row.id,
      kind: row.kind,
      severity,
      reason: rowReason,
      ship: reference(row.ship),
      station: typeof row.station === 'string' ? row.station : null,
      first_seen_tick: Number.isSafeInteger(row.first_seen_tick) ? row.first_seen_tick : 0,
    });
  }

  // `boundary_tick` is absent when the split was refused before any boundary was
  // agreed, and stays absent here: null, never a tick nothing ever held at.
  const recovery = value.recovery && typeof value.recovery === 'object' ? {
    divergence_tick: Number(value.recovery.divergence_tick) || 0,
    boundary_tick: Number.isSafeInteger(value.recovery.boundary_tick)
      ? value.recovery.boundary_tick
      : null,
    recovering_peers: Number(value.recovery.recovering_peers) || 0,
    failed: value.recovery.failed === true,
  } : null;

  // The live restore (issue #1446), carried through unchanged for
  // `gui/gm-restore-control.js` to read: this adapter validates the payload
  // shape and does not interpret a phase.
  const restore = value.restore && typeof value.restore === 'object' ? { ...value.restore } : null;

  return {
    tick,
    paused: value.paused === true,
    input_delay_ticks: Number.isSafeInteger(value.input_delay_ticks) ? value.input_delay_ticks : null,
    recovery,
    restore,
    peers,
    stations,
    operators,
    alerts,
  };
}

/** The worst state anything in a parsed projection is in. */
export function worstHealthState(projection) {
  if (!projection) return 'live';
  const rows = [
    ...projection.peers.map((row) => row.state),
    ...projection.stations.map((row) => row.state),
    ...projection.operators.map((row) => row.state),
    ...projection.alerts.map((row) => row.severity),
    projection.paused ? 'paused' : 'live',
  ];
  return rows.reduce((worst, row) => (healthStateRank(row) > healthStateRank(worst) ? row : worst), 'live');
}

/**
 * Build the technical banner renderer.
 *
 * @param {{
 *   doc?: Document,
 *   t?: (id: string, params?: object) => string,
 *   onAction?: (alert: object) => void,
 * }} opts
 */
export function createGmHealthBanner({
  doc = globalThis.document,
  t = (id) => id,
  onAction = () => {},
} = {}) {
  /** Signature of what is currently drawn per banner id, so an unchanged row is
   * left alone (and keeps whatever focus is standing on it). */
  const drawn = new Map();

  function signature(alert) {
    return JSON.stringify([alert.severity, alert.reason, alert.ship, alert.kind]);
  }

  /**
   * One banner element. Also the `renderBanner` hook the #1433 attention panel
   * takes, so the region it guards and the component that fills it stay
   * separate concerns.
   */
  function element(alert) {
    const item = doc.createElement('p');
    item.className = 'gm-health-banner';
    item.dataset.bannerId = alert.id;
    item.dataset.state = alert.severity;
    item.dataset.kind = alert.kind;
    // The state, in words, first: colour and the data attribute are
    // decoration, and forced colours removes both.
    const state = doc.createElement('span');
    state.className = 'gm-health-banner-state';
    state.textContent = t(healthStateLabelId(alert.severity));
    const message = doc.createElement('span');
    message.className = 'gm-health-banner-message';
    message.textContent = t(alert.reason.id, alert.reason.params);
    item.append(state, message);
    // Actionable, always: a failure a facilitator can only read is a failure
    // they have to go and hunt for. A hull-scoped alert takes them to the hull;
    // everything else takes them to the health panel that explains it.
    const action = doc.createElement('button');
    action.type = 'button';
    action.dataset.action = 'focus';
    action.textContent = alert.ship
      ? t('server.gm.health.focus_ship', { ship: alert.ship.name })
      : t('server.gm.health.focus_panel');
    action.addEventListener('click', () => onAction(alert));
    item.append(action);
    return item;
  }

  /**
   * Draw `alerts` into `container`, verbatim and in the order given.
   *
   * No filter, no snooze, no hold: this entry point takes rows and draws rows,
   * which is the whole guarantee. Rows already on screen with unchanged content
   * are reused so a button under the keyboard survives the repaint.
   *
   * @returns the number of banners now showing.
   */
  function render(alerts, container) {
    if (!container) return 0;
    const list = (Array.isArray(alerts) ? alerts : []).filter(
      (alert) => alert && typeof alert.id === 'string' && alert.reason && typeof alert.reason.id === 'string',
    );
    const wanted = new Map(list.map((alert) => [alert.id, alert]));
    for (const [id, node] of [...drawn]) {
      if (!wanted.has(id)) { node.element.remove(); drawn.delete(id); }
    }
    const order = [];
    for (const alert of list) {
      const existing = drawn.get(alert.id);
      const next = signature(alert);
      if (existing && existing.signature === next) {
        order.push(existing.element);
        continue;
      }
      const node = element(alert);
      if (existing) existing.element.replaceWith(node);
      drawn.set(alert.id, { signature: next, element: node });
      order.push(node);
    }
    // Reorder ONLY when the order actually changed: taking a node out of the
    // document to put it back in the same place still blurs whatever was
    // focused inside it, which would undo the reuse above.
    const current = [...container.children];
    const settled = current.length === order.length && current.every((node, at) => node === order[at]);
    if (!settled) for (const node of order) container.append(node);
    container.hidden = order.length === 0;
    return order.length;
  }

  return {
    element,
    render,
    /** Forget everything drawn, for a desk that is being torn down or reset. */
    reset(container) {
      drawn.clear();
      if (container) { container.replaceChildren(); container.hidden = true; }
    },
    state: () => ({ drawn: [...drawn.keys()] }),
  };
}
