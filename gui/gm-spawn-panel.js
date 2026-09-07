/**
 * The GM placement panel: the scenario-authored spawn palette and the map
 * gesture that places one (issue #1305, PRD #930 milestone M2).
 *
 * Pure over injected page/transport/map seams, like every other GM surface
 * here — no globals, no WASM, no timers of its own beyond the injected
 * scheduler — so vitest drives the whole lifecycle against a plain DOM and a
 * stub map.
 *
 * # Where a placement comes from
 *
 * The chart component owns the GESTURE (press picks the position, drag picks
 * the heading, arrows-and-brackets-and-Enter do the same from the keyboard) and
 * emits one `navplace` carrying resolved world `{x, z, heading}` in metres and
 * degrees. This module owns the COMMAND: it pairs that placement with the
 * palette row the operator armed, converts metres to the wire's fixed point
 * once, and submits. Pixels never reach it, and neither mouse nor touch has a
 * path of its own — they are the same `navplace`.
 *
 * The typed form beside the map is the same command from the other direction:
 * an operator who cannot drag types the coordinates, and the identical
 * `place()` runs. There is one submit path, not one per input device.
 *
 * # The projection is absolute
 *
 * `update()` replaces the palette and the result feed outright, so a GM that
 * reconnects, or whose peer restored from a snapshot, sees exactly what a live
 * one sees. The only local state is the operator's own in-flight placements and
 * which palette row is armed.
 */

import {
  ACTION_FEEDBACK_STATE,
  ActionFeedbackLifecycle,
  DEFAULT_ACTION_FEEDBACK_TIMEOUT_MS,
  createActionCorrelation,
} from './action-feedback.js';
import {
  GM_ACTION_REFUSAL_REASON_LABELS,
  LOCAL_INGRESS_REFUSAL,
} from './gm-action-reasons.js';

export const GM_SPAWN_FEED_CAPACITY = 32;

/** The semantic-action id family a placement reports feedback under. */
export const GM_PLACE_ACTION_PREFIX = 'gm.spawn.place:';

/**
 * Millimetres per metre and millidegrees per degree — the browser half of the
 * fixed point `src/gm_spawn.rs` documents. One scale, written once.
 */
export const GM_SPAWN_FIXED_POINT_SCALE = 1000;

/** The action's own coordinate bound, mirrored from `MAX_GM_SPAWN_COORD_MM`. */
export const GM_SPAWN_MAX_COORD_MM = 5_000_000_000;

const RESULT_OUTCOMES = new Set(['applied', 'no-op', 'refused']);

const FEEDBACK_STATUS_IDS = Object.freeze({
  [ACTION_FEEDBACK_STATE.PENDING]: 'action_feedback.pending',
  [ACTION_FEEDBACK_STATE.APPLIED]: 'action_feedback.applied',
  [ACTION_FEEDBACK_STATE.REFUSED]: 'action_feedback.refused',
  [ACTION_FEEDBACK_STATE.TIMED_OUT]: 'action_feedback.timed_out',
});

function parseVariant(value) {
  if (!value || typeof value !== 'object'
      || typeof value.id !== 'string' || value.id.length === 0
      || typeof value.label !== 'string' || value.label.length === 0) return null;
  return { id: value.id, label: value.label };
}

function parseEntry(value) {
  if (!value || typeof value !== 'object'
      || typeof value.id !== 'string' || value.id.length === 0
      || typeof value.label !== 'string' || value.label.length === 0
      || !Array.isArray(value.variants)) return null;
  const variants = [];
  for (const candidate of value.variants) {
    const parsed = parseVariant(candidate);
    if (!parsed) return null;
    variants.push(parsed);
  }
  return { id: value.id, label: value.label, variants };
}

function parseResult(value) {
  if (!value || typeof value !== 'object'
      || typeof value.operator_id !== 'string' || value.operator_id.length === 0
      || typeof value.correlation !== 'string' || value.correlation.length === 0
      || !RESULT_OUTCOMES.has(value.outcome)
      || !Number.isSafeInteger(value.tick) || value.tick < 0
      || (value.reason != null && typeof value.reason !== 'string')
      || (value.target != null && typeof value.target !== 'string')) return null;
  return {
    operator_id: value.operator_id,
    correlation: value.correlation,
    outcome: value.outcome,
    tick: value.tick,
    ...(value.reason ? { reason: value.reason } : {}),
    ...(value.target ? { target: value.target } : {}),
  };
}

/**
 * Parse one absolute Host Channel projection without retaining partial data.
 *
 * A malformed row rejects the WHOLE payload, for the mission panel's reason: a
 * panel that quietly dropped one palette row would offer a GM an incomplete
 * vocabulary with no indication that it had done so.
 */
export function parseGmSpawnPayload(payload) {
  let value = payload;
  if (typeof value === 'string') {
    try { value = JSON.parse(value); } catch (_) { return undefined; }
  }
  if (!value || typeof value !== 'object'
      || !Array.isArray(value.palette) || !Array.isArray(value.results)) return undefined;
  const palette = [];
  for (const candidate of value.palette) {
    const parsed = parseEntry(candidate);
    if (!parsed) return undefined;
    palette.push(parsed);
  }
  const results = [];
  for (const candidate of value.results) {
    const parsed = parseResult(candidate);
    if (!parsed) return undefined;
    results.push(parsed);
  }
  return { palette, results };
}

/**
 * Convert one resolved world placement to the wire's fixed point, or
 * `undefined` when it is not a placement at all.
 *
 * The one metres-to-millimetres conversion in the browser, applying the same
 * bound `gm_spawn::placement_is_valid` applies, so an out-of-range gesture is
 * refused HERE rather than becoming a request the simulation must reject.
 */
export function placementToWire(placement) {
  if (!placement || typeof placement !== 'object') return undefined;
  const { x, z, heading } = placement;
  const y = Number.isFinite(placement.y) ? placement.y : 0;
  if (!Number.isFinite(x) || !Number.isFinite(z) || !Number.isFinite(heading)) return undefined;
  const position_mm = [x, y, z].map((axis) => Math.round(axis * GM_SPAWN_FIXED_POINT_SCALE));
  if (position_mm.some((axis) => !Number.isSafeInteger(axis)
      || Math.abs(axis) > GM_SPAWN_MAX_COORD_MM)) return undefined;
  const normalised = ((heading % 360) + 360) % 360;
  return { position_mm, heading_mdeg: Math.round(normalised * GM_SPAWN_FIXED_POINT_SCALE) };
}

function setAccessibility(region, heading, list, empty, feedback, log) {
  if (region) {
    region.setAttribute('role', 'region');
    if (heading) region.setAttribute('aria-labelledby', heading.id);
  }
  if (list) list.setAttribute('role', 'list');
  if (empty) {
    empty.setAttribute('role', 'status');
    empty.setAttribute('aria-live', 'polite');
  }
  if (feedback) {
    feedback.setAttribute('role', 'status');
    feedback.setAttribute('aria-live', 'polite');
    feedback.setAttribute('aria-atomic', 'true');
  }
  if (log) {
    log.setAttribute('role', 'log');
    log.setAttribute('aria-live', 'polite');
    log.setAttribute('aria-relevant', 'additions text');
  }
}

function entryKey(operatorId, correlation) {
  return JSON.stringify([operatorId, correlation]);
}

/** Mount the GM placement panel over injected page/transport/map seams. */
export function createGmSpawnPanel({
  doc = globalThis.document,
  win = doc && doc.defaultView,
  t = (id) => id,
  submitPlacement = null,
  confirmAction = (request) => request.accept(),
  getOperator = () => null,
  getOperatorName = (id) => id,
  getMap = () => (doc ? doc.getElementById('gm-entity-map') : null),
  correlation = createActionCorrelation,
  now,
  capacity = GM_SPAWN_FEED_CAPACITY,
  timeoutMs = DEFAULT_ACTION_FEEDBACK_TIMEOUT_MS,
  schedule = (fn, delay) => setTimeout(fn, delay),
  cancelSchedule = (timer) => clearTimeout(timer),
  actionFeedback: suppliedActionFeedback = null,
} = {}) {
  const region = doc && doc.getElementById('gm-spawn-panel');
  const heading = doc && doc.getElementById('gm-spawn-heading');
  const list = doc && doc.getElementById('gm-spawn-palette');
  const empty = doc && doc.getElementById('gm-spawn-empty');
  const feedbackStatus = doc && doc.getElementById('gm-spawn-feedback');
  const log = doc && doc.getElementById('gm-spawn-log');
  const exactX = doc && doc.getElementById('gm-spawn-x');
  const exactZ = doc && doc.getElementById('gm-spawn-z');
  const exactHeading = doc && doc.getElementById('gm-spawn-facing');
  const exactButton = doc && doc.getElementById('gm-spawn-exact');
  const boundedCapacity = Math.max(
    1,
    Number.isInteger(capacity) ? capacity : GM_SPAWN_FEED_CAPACITY,
  );
  const boundedTimeoutMs = Number.isFinite(timeoutMs) && timeoutMs >= 0
    ? timeoutMs : DEFAULT_ACTION_FEEDBACK_TIMEOUT_MS;

  setAccessibility(region, heading, list, empty, feedbackStatus, log);

  let palette = [];
  let authoritativeResults = [];
  let armedPaletteId = null;
  const selectedVariants = new Map();
  const pending = new Map();
  const localTerminals = new Map();
  const buttons = new Map();
  const selects = new Map();
  let boundMap = null;

  const actionFeedback = suppliedActionFeedback || new ActionFeedbackLifecycle({
    ...(typeof correlation === 'function' ? { correlation } : {}),
    ...(typeof now === 'function' ? { now } : {}),
    capacity: boundedCapacity,
  });

  function operator() {
    try {
      const value = typeof getOperator === 'function' ? getOperator() : null;
      return value && typeof value.id === 'string' && value.id.length > 0 ? value : null;
    } catch (_) {
      return null;
    }
  }

  function operatorName(id) {
    try {
      const value = typeof getOperatorName === 'function' ? getOperatorName(id) : id;
      return typeof value === 'string' && value.length > 0 ? value : id;
    } catch (_) {
      return id;
    }
  }

  function map() {
    try {
      return typeof getMap === 'function' ? getMap() : null;
    } catch (_) {
      return null;
    }
  }

  function entryLabel(paletteId) {
    const entry = palette.find((candidate) => candidate.id === paletteId);
    return entry ? t(entry.label) : paletteId || '';
  }

  function refusalText(reason) {
    if (!reason) return t('server.gm.spawn.reason_unspecified');
    if (reason === LOCAL_INGRESS_REFUSAL) {
      return t('server.gm.session.reason.ingress_rejected');
    }
    const labelId = GM_ACTION_REFUSAL_REASON_LABELS[reason];
    return labelId ? t(labelId) : t('server.gm.spawn.reason_unknown', { reason });
  }

  function clearPendingTimer(meta) {
    if (!meta || meta.timerScheduled !== true) return;
    try { cancelSchedule(meta.timer); } catch (_) { /* timer already completed */ }
    meta.timer = null;
    meta.timerScheduled = false;
  }

  function appendRow(operatorId, correlationValue) {
    if (!log) return null;
    const row = doc.createElement('li');
    row.className = 'gm-spawn-log-entry';
    row.dataset.entryKey = entryKey(operatorId, correlationValue);
    row.dataset.operatorId = operatorId;
    row.dataset.correlation = correlationValue;
    log.appendChild(row);
    return row;
  }

  function paintResultRow(result) {
    const row = appendRow(result.operator_id, result.correlation);
    if (!row) return;
    const suffix = result.outcome === 'no-op' ? 'no_op' : result.outcome;
    row.dataset.outcome = result.outcome;
    row.dataset.tick = String(result.tick);
    if (result.target) row.dataset.palette = result.target;
    if (result.reason) row.dataset.reason = result.reason;
    row.textContent = t(`server.gm.spawn.result_${suffix}`, {
      name: operatorName(result.operator_id),
      entry: entryLabel(result.target),
      tick: String(result.tick),
      correlation: result.correlation,
      reason: refusalText(result.reason),
    });
  }

  function paintLocalRow(meta, outcome, reason) {
    const row = appendRow(meta.operatorId, meta.correlation);
    if (!row) return;
    row.dataset.outcome = outcome;
    row.dataset.palette = meta.palette;
    if (reason) row.dataset.reason = reason;
    const statusId = {
      pending: 'server.gm.spawn.result_pending',
      'timed-out': 'server.gm.spawn.result_timed_out',
      refused: 'server.gm.spawn.result_refused_local',
    }[outcome];
    row.textContent = t(statusId, {
      name: meta.operatorName,
      entry: entryLabel(meta.palette),
      correlation: meta.correlation,
      reason: refusalText(reason),
    });
  }

  /** Rebuild deterministically: absolute terminal order, then local live rows. */
  function renderLog() {
    if (!log) return;
    log.replaceChildren();
    const authoritativeKeys = new Set();
    for (const result of authoritativeResults) {
      authoritativeKeys.add(entryKey(result.operator_id, result.correlation));
      paintResultRow(result);
    }
    for (const [key, terminal] of localTerminals) {
      if (!authoritativeKeys.has(key)) {
        paintLocalRow(terminal, terminal.outcome, terminal.reason);
      }
    }
    for (const meta of pending.values()) {
      if (!authoritativeKeys.has(entryKey(meta.operatorId, meta.correlation))) {
        paintLocalRow(meta, 'pending', null);
      }
    }
  }

  function paintFeedback(state, paletteId) {
    if (!feedbackStatus) return;
    feedbackStatus.dataset.state = state || '';
    feedbackStatus.dataset.palette = paletteId || '';
    const statusId = FEEDBACK_STATUS_IDS[state];
    feedbackStatus.textContent = statusId
      ? t('action_feedback.summary', {
          action: t('server.gm.spawn.place_accessibility', { label: entryLabel(paletteId) }),
          status: t(statusId),
        })
      : '';
  }

  function rememberLocalTerminal(meta, outcome, reason) {
    localTerminals.set(entryKey(meta.operatorId, meta.correlation), {
      ...meta,
      outcome,
      reason,
    });
    while (localTerminals.size > boundedCapacity) {
      localTerminals.delete(localTerminals.keys().next().value);
    }
  }

  function finishLocalPending(correlationValue, outcome, reason) {
    const meta = pending.get(correlationValue);
    if (!meta) return false;
    clearPendingTimer(meta);
    pending.delete(correlationValue);
    rememberLocalTerminal(meta, outcome, reason);
    paintFeedback(
      outcome === 'timed-out' ? ACTION_FEEDBACK_STATE.TIMED_OUT : ACTION_FEEDBACK_STATE.REFUSED,
      meta.palette,
    );
    renderLog();
    refreshAdmission();
    return true;
  }

  /**
   * Submit one resolved placement of one palette entry.
   *
   * The single command path: the map gesture, the touch gesture and the typed
   * form all arrive here with world metres and degrees already resolved.
   */
  function place(paletteId, placement) {
    const current = operator();
    const entry = palette.find((candidate) => candidate.id === paletteId);
    const wire = placementToWire(placement);
    if (!current || !entry || !wire) return false;
    const variant = selectedVariants.get(paletteId) || null;
    if (variant !== null && !entry.variants.some((candidate) => candidate.id === variant)) {
      return false;
    }
    const description = t('settings.gm.confirmation.spawn', {
      name: t(entry.label), x: placement.x, z: placement.z,
    });
    return confirmAction({ category: 'world.spawn', description, preview: () => description,
      accept: () => submitPlacementIntent(current, paletteId, variant, wire),
    });
  }

  function submitPlacementIntent(current, paletteId, variant, wire) {
    if (operator()?.id !== current.id) return false;
    while (pending.size >= boundedCapacity) {
      const oldest = pending.keys().next().value;
      if (oldest === undefined) break;
      finishLocalPending(oldest, 'timed-out', null);
    }
    const press = actionFeedback.press(`${GM_PLACE_ACTION_PREFIX}${paletteId}`);
    const meta = {
      palette: paletteId,
      variant,
      correlation: press.correlation,
      operatorId: current.id,
      operatorName: typeof current.name === 'string' && current.name.length > 0
        ? current.name : operatorName(current.id),
      timer: null,
      timerScheduled: false,
    };
    pending.set(press.correlation, meta);
    let accepted = false;
    try {
      accepted = typeof submitPlacement === 'function'
        && submitPlacement({
          palette: paletteId,
          variant,
          position_mm: wire.position_mm,
          heading_mdeg: wire.heading_mdeg,
          correlation: press.correlation,
        }) !== false;
    } catch (_) {
      accepted = false;
    }
    actionFeedback.pending(press.correlation);
    if (!accepted) {
      actionFeedback.settle(press.correlation, ACTION_FEEDBACK_STATE.REFUSED);
      finishLocalPending(press.correlation, 'refused', LOCAL_INGRESS_REFUSAL);
      return true;
    }
    meta.timerScheduled = true;
    meta.timer = schedule(() => {
      actionFeedback.settle(meta.correlation, ACTION_FEEDBACK_STATE.TIMED_OUT);
      finishLocalPending(meta.correlation, 'timed-out', null);
    }, boundedTimeoutMs);
    paintFeedback(ACTION_FEEDBACK_STATE.PENDING, paletteId);
    renderLog();
    refreshAdmission();
    return true;
  }

  function setArmed(paletteId) {
    armedPaletteId = paletteId;
    if (region) region.dataset.arming = paletteId || '';
    const chart = map();
    if (chart) {
      if (paletteId && typeof chart.navigationBeginPlacement === 'function') {
        chart.navigationBeginPlacement();
      } else if (!paletteId && typeof chart.navigationCancelPlacement === 'function') {
        chart.navigationCancelPlacement();
      }
    }
    refreshAdmission();
  }

  /** Arm (or disarm) the map gesture for one palette entry. */
  function arm(paletteId) {
    const entry = palette.find((candidate) => candidate.id === paletteId);
    if (!entry || !operator()) return false;
    setArmed(armedPaletteId === paletteId ? null : paletteId);
    return true;
  }

  function onPlaced(event) {
    const detail = event && event.detail;
    if (!armedPaletteId || !detail) return;
    const paletteId = armedPaletteId;
    armedPaletteId = null;
    if (region) region.dataset.arming = '';
    place(paletteId, detail);
    refreshAdmission();
  }

  function onPlaceCancelled() {
    armedPaletteId = null;
    if (region) region.dataset.arming = '';
    refreshAdmission();
  }

  function bindMap() {
    const chart = map();
    if (chart === boundMap) return;
    if (boundMap) {
      boundMap.removeEventListener('navplace', onPlaced);
      boundMap.removeEventListener('navplacecancel', onPlaceCancelled);
    }
    boundMap = chart;
    if (boundMap) {
      boundMap.addEventListener('navplace', onPlaced);
      boundMap.addEventListener('navplacecancel', onPlaceCancelled);
    }
  }

  function onPlaceClick(event) {
    if (event && typeof event.preventDefault === 'function') event.preventDefault();
    const target = event && event.currentTarget;
    if (target && target.dataset && target.dataset.paletteId) arm(target.dataset.paletteId);
  }

  function onVariantChange(event) {
    const target = event && event.currentTarget;
    if (!target || !target.dataset || !target.dataset.paletteId) return;
    selectedVariants.set(
      target.dataset.paletteId,
      target.value && target.value.length > 0 ? target.value : null,
    );
  }

  /**
   * The typed placement path: the same command with the numbers read out of
   * the form instead of resolved from a gesture. Present so an operator who
   * cannot drag — or cannot use a pointer at all — reaches every placement the
   * chart reaches.
   */
  function exactTarget() {
    return palette.find((candidate) => candidate.id === armedPaletteId) || palette[0] || null;
  }

  function placeExact() {
    const entry = exactTarget();
    if (!entry) return false;
    const read = (input) => (input ? Number(input.value) : Number.NaN);
    const placed = place(entry.id, {
      x: read(exactX),
      z: read(exactZ),
      heading: read(exactHeading),
    });
    if (placed) setArmed(null);
    return placed;
  }

  function onExactClick(event) {
    if (event && typeof event.preventDefault === 'function') event.preventDefault();
    placeExact();
  }

  function renderPalette() {
    if (!list) return;
    for (const button of buttons.values()) button.removeEventListener('click', onPlaceClick);
    for (const select of selects.values()) select.removeEventListener('change', onVariantChange);
    list.replaceChildren();
    buttons.clear();
    selects.clear();
    for (const entry of palette) {
      const row = doc.createElement('li');
      row.className = 'gm-spawn-entry';
      row.dataset.paletteId = entry.id;
      const label = doc.createElement('span');
      label.className = 'gm-spawn-entry-label';
      label.textContent = t(entry.label);
      row.append(label);
      if (entry.variants.length > 0) {
        const select = doc.createElement('select');
        select.className = 'gm-spawn-entry-variant';
        select.dataset.paletteId = entry.id;
        select.setAttribute(
          'aria-label',
          t('server.gm.spawn.variant_accessibility', { label: t(entry.label) }),
        );
        const bare = doc.createElement('option');
        bare.value = '';
        bare.textContent = t('server.gm.spawn.variant_none');
        select.append(bare);
        for (const variant of entry.variants) {
          const option = doc.createElement('option');
          option.value = variant.id;
          option.textContent = t(variant.label);
          select.append(option);
        }
        // An unknown id simply does not stick, leaving the bare template
        // selected — the safe reading of a palette row whose variants moved.
        const chosen = selectedVariants.get(entry.id);
        select.value = typeof chosen === 'string' ? chosen : '';
        select.addEventListener('change', onVariantChange);
        selects.set(entry.id, select);
        row.append(select);
      }
      const button = doc.createElement('button');
      button.type = 'button';
      button.dataset.role = 'place';
      button.dataset.paletteId = entry.id;
      button.textContent = t('server.gm.spawn.place');
      button.setAttribute(
        'aria-label',
        t('server.gm.spawn.place_accessibility', { label: t(entry.label) }),
      );
      button.addEventListener('click', onPlaceClick);
      buttons.set(entry.id, button);
      row.append(button);
      list.append(row);
    }
    if (empty) {
      empty.hidden = palette.length > 0;
      empty.textContent = palette.length > 0 ? '' : t('server.gm.spawn.empty');
    }
  }

  function refreshAdmission() {
    const admitted = !!operator();
    if (region) region.dataset.admitted = String(admitted);
    for (const [paletteId, button] of buttons) {
      button.disabled = !admitted;
      button.setAttribute('aria-disabled', admitted ? 'false' : 'true');
      button.dataset.arming = String(armedPaletteId === paletteId);
    }
    if (exactButton) {
      // The typed form acts on the armed row, falling back to the first — so
      // which row it will place is never a guess: it is on the button, in the
      // DOM, and in the accessible name beside the coordinates.
      const target = exactTarget();
      const enabled = admitted && !!target;
      exactButton.disabled = !enabled;
      exactButton.setAttribute('aria-disabled', enabled ? 'false' : 'true');
      exactButton.dataset.paletteId = target ? target.id : '';
      exactButton.setAttribute(
        'aria-label',
        target
          ? t('server.gm.spawn.place_accessibility', { label: t(target.label) })
          : t('server.gm.spawn.exact_submit'),
      );
    }
    return admitted;
  }

  function update(payload) {
    const projection = parseGmSpawnPayload(payload);
    if (!projection) return false;
    palette = projection.palette;
    // A palette row that has gone takes its variant choice and any arming with
    // it, and so does a VARIANT that has gone from a surviving row — a layer
    // authoring one can unload under an operator who had already chosen it.
    // Dropping the stale choice falls back to the bare template, which is what
    // the rebuilt select shows; keeping it would leave the row rendering as
    // placeable while every press refused with nothing on screen to explain it.
    for (const [paletteId, variant] of [...selectedVariants]) {
      const entry = palette.find((candidate) => candidate.id === paletteId);
      if (!entry || (variant !== null
          && !entry.variants.some((candidate) => candidate.id === variant))) {
        selectedVariants.delete(paletteId);
      }
    }
    if (armedPaletteId && !palette.some((entry) => entry.id === armedPaletteId)) {
      setArmed(null);
    }
    authoritativeResults = projection.results.slice(-boundedCapacity);
    localTerminals.clear();
    for (const result of projection.results) {
      const meta = pending.get(result.correlation);
      if (!meta || meta.operatorId !== result.operator_id) continue;
      clearPendingTimer(meta);
      pending.delete(result.correlation);
      const state = result.outcome === 'refused'
        ? ACTION_FEEDBACK_STATE.REFUSED
        : ACTION_FEEDBACK_STATE.APPLIED;
      actionFeedback.settle(result.correlation, state);
      paintFeedback(state, meta.palette);
    }
    bindMap();
    renderPalette();
    renderLog();
    refreshAdmission();
    return true;
  }

  /** Explicit run boundary, called from the authoritative Lobby transition. */
  function reset() {
    for (const meta of pending.values()) {
      clearPendingTimer(meta);
      actionFeedback.cancel(meta.correlation);
    }
    pending.clear();
    localTerminals.clear();
    selectedVariants.clear();
    authoritativeResults = [];
    palette = [];
    setArmed(null);
    renderPalette();
    if (log) log.replaceChildren();
    paintFeedback(null, null);
    refreshAdmission();
  }

  if (exactButton) exactButton.addEventListener('click', onExactClick);
  bindMap();
  renderPalette();
  refreshAdmission();

  function destroy() {
    for (const button of buttons.values()) button.removeEventListener('click', onPlaceClick);
    for (const select of selects.values()) select.removeEventListener('change', onVariantChange);
    buttons.clear();
    selects.clear();
    if (exactButton) exactButton.removeEventListener('click', onExactClick);
    if (boundMap) {
      boundMap.removeEventListener('navplace', onPlaced);
      boundMap.removeEventListener('navplacecancel', onPlaceCancelled);
      boundMap = null;
    }
    for (const meta of pending.values()) clearPendingTimer(meta);
    pending.clear();
  }

  return {
    actionFeedback,
    arm,
    place,
    placeExact,
    update,
    reset,
    refreshAdmission,
    state: () => ({
      palette: palette.length,
      arming: armedPaletteId,
      variants: Object.fromEntries(selectedVariants),
      pending: pending.size,
      authoritative: authoritativeResults.length,
    }),
    destroy,
    win,
  };
}
