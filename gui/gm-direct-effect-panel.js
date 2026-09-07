/**
 * Direct damage and repair on the selected entity (issue #1310, PRD #930
 * milestone M2 Directing).
 *
 * Pure over injected page/transport seams, like every other GM surface here —
 * no globals, no WASM, no timers of its own beyond the injected scheduler — so
 * vitest drives the whole lifecycle against a plain DOM.
 *
 * # Why this lives on the inspector rather than in the mission panel
 *
 * A Fire names an authored event out of a list the scenario published. A
 * direct effect names an ENTITY, and the one place a GM picks an entity is the
 * map/inspector selection this panel reads. Its authoritative results ride the
 * same `gm_entity` projection for the same reason: the answer belongs beside
 * the thing it was aimed at, and a second Host Channel carrying four rows
 * would be a second place for the two to drift out of step.
 *
 * # The amount is milli-HP end to end
 *
 * The operator types hull points; everything below the input carries
 * thousandths of a point, exactly as `GmAction::ApplyDirectEffect` does. No
 * layer re-derives hull points from a percentage, which is what keeps the
 * preview and the authoritative result talking about the same numbers.
 *
 * # The preview is a preview
 *
 * `lethal` and `discarded` here are computed from the projection this browser
 * last received. The AUTHORITATIVE answer is resolved again in Rust at the
 * agreed apply tick and comes back on the result row; a warning that turns out
 * to have been stale is a warning, never a decision.
 *
 * # Scope narrows the same press (issue #1311)
 *
 * The Scope picker chooses the whole entity, one Station, or one System out of
 * the per-System breakdown the `gm_entity` projection publishes. The browser
 * never composes a Station or System identity of its own, for the reason it
 * never composes an entity one: an authoring key the simulation did not publish
 * is a key the apply tick would have to refuse.
 *
 * Narrowing changes exactly two things about the preview, and they are the two
 * things it changes in Rust. The CLAMP measures the scope, so overflow is what
 * that Station cannot absorb; LETHALITY still measures the whole hull, because
 * emptying a Station is not sinking a ship.
 */

import { wireText } from './strings.js';
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
import {
  gmEffectScopeFromKey,
  gmEffectScopeKey,
  gmEffectScopeLabel,
  gmEffectScopeOptions,
  gmEffectScopeRequestFields,
  gmEffectScopeTotals,
  parseGmEffectScope,
} from './gm-effect-scope.js';

export const GM_EFFECT_FEED_CAPACITY = 32;

/** The semantic-action id family a direct-effect press reports feedback under. */
export const GM_EFFECT_ACTION_PREFIX = 'gm.effect.apply:';

/** Milli-HP per hull point — the one place the unit conversion is spelled. */
export const MILLI_HP_PER_HP = 1000;

const RESULT_OUTCOMES = new Set(['applied', 'no-op', 'refused']);
const EFFECT_KINDS = new Set(['damage', 'heal']);

const FEEDBACK_STATUS_IDS = Object.freeze({
  [ACTION_FEEDBACK_STATE.PENDING]: 'action_feedback.pending',
  [ACTION_FEEDBACK_STATE.APPLIED]: 'action_feedback.applied',
  [ACTION_FEEDBACK_STATE.REFUSED]: 'action_feedback.refused',
  [ACTION_FEEDBACK_STATE.TIMED_OUT]: 'action_feedback.timed_out',
});

/** Hull points from milli-HP, without trailing zeroes. */
export function hullPoints(milliHp) {
  if (!Number.isFinite(milliHp)) return '0';
  return String(Math.round(milliHp) / MILLI_HP_PER_HP);
}

/** Milli-HP from a typed hull-point amount, or null when it is not a request. */
export function milliHpFromInput(value) {
  const points = typeof value === 'string' ? Number(value.trim()) : Number(value);
  if (!Number.isFinite(points) || points <= 0) return null;
  const milli = Math.round(points * MILLI_HP_PER_HP);
  return milli > 0 && Number.isSafeInteger(milli) ? milli : null;
}

function parseEffectResult(value) {
  if (!value || typeof value !== 'object'
      || typeof value.operator_id !== 'string' || value.operator_id.length === 0
      || typeof value.correlation !== 'string' || value.correlation.length === 0
      || !RESULT_OUTCOMES.has(value.outcome)
      || !Number.isSafeInteger(value.tick) || value.tick < 0
      || (value.reason != null && typeof value.reason !== 'string')
      || (value.target != null && typeof value.target !== 'string')) return null;
  // `effect_scope`, because that is what `LoggedGmAction` calls it — this feed
  // IS that struct, and its sibling is `effect`. (The activity feed's row is a
  // different DTO whose field is `scope`; both go through the one parser.)
  //
  // A malformed scope rejects the WHOLE row rather than being dropped: a result
  // that quietly lost its narrowing would tell a GM they emptied a ship when
  // they emptied one Station.
  const scope = parseGmEffectScope(value.effect_scope);
  if (scope === undefined) return null;
  let effect;
  if (value.effect != null) {
    const source = value.effect;
    if (!source || typeof source !== 'object'
        || !EFFECT_KINDS.has(source.kind)
        || !Number.isSafeInteger(source.applied_milli_hp) || source.applied_milli_hp < 0
        || !Number.isSafeInteger(source.discarded_milli_hp) || source.discarded_milli_hp < 0
        || typeof source.destroyed !== 'boolean') return null;
    effect = {
      kind: source.kind,
      applied_milli_hp: source.applied_milli_hp,
      discarded_milli_hp: source.discarded_milli_hp,
      destroyed: source.destroyed,
    };
  }
  return {
    operator_id: value.operator_id,
    correlation: value.correlation,
    outcome: value.outcome,
    tick: value.tick,
    ...(value.reason ? { reason: value.reason } : {}),
    ...(value.target ? { target: value.target } : {}),
    ...(scope ? { scope } : {}),
    ...(effect ? { effect } : {}),
  };
}

/**
 * Parse the directed-effect half of one absolute `gm_entity` payload.
 *
 * A malformed row rejects the WHOLE list for `parseGmMissionPayload`'s reason:
 * a feed that quietly dropped one attributed result would tell a GM their
 * press vanished.
 */
export function parseGmEffectResults(payload) {
  let value = payload;
  if (typeof value === 'string') {
    try { value = JSON.parse(value); } catch (_) { return undefined; }
  }
  if (!value || typeof value !== 'object' || !Array.isArray(value.results)) return undefined;
  const results = [];
  for (const candidate of value.results) {
    const parsed = parseEffectResult(candidate);
    if (!parsed) return undefined;
    results.push(parsed);
  }
  return results;
}

/**
 * What a press of `amount` would do to `entity`, from the last projection.
 *
 * The shape of `gm_effect::resolve_direct_effect`, deliberately: the warning a
 * GM reads before pressing has to be the same statement the authoritative
 * reducer will make, or the preview is noise. It is not the same ARITHMETIC to
 * the last milli-HP — Rust rounds the clamp up so a lethal promise is always
 * keepable, while the projection carries rounded totals — and it does not need
 * to be: the tick this resolves at has not happened yet either.
 */
export function previewDirectEffect(entity, kind, amountMilliHp, scope = null) {
  const totals = gmEffectScopeTotals(entity, scope);
  const hull = gmEffectScopeTotals(entity, null);
  if (!totals || !hull
      || !Number.isSafeInteger(amountMilliHp) || amountMilliHp <= 0) return null;
  const headroom = kind === 'heal'
    ? Math.max(0, totals.max - totals.current)
    : totals.current;
  const applied = Math.min(amountMilliHp, headroom);
  return {
    applied_milli_hp: applied,
    discarded_milli_hp: amountMilliHp - applied,
    // Emptying the SCOPE is `emptied`; emptying the HULL is `destroyed`. Rust
    // draws the line in exactly this place (`resolve_direct_effect_within`),
    // and a preview that collapsed the two would promise a kill the world then
    // refuses to perform.
    emptied: kind === 'damage' && applied > 0 && applied === headroom,
    destroyed: kind === 'damage' && applied > 0 && applied >= hull.current,
  };
}

/** Whether an entity can accept a direct effect at all. */
export function entityIsDamageable(entity) {
  return !!gmEffectScopeTotals(entity, null);
}

/**
 * Whether a SCOPE on that entity can accept one (issue #1311).
 *
 * A Station whose Systems this hull tracks none of, and a System the hull does
 * not track, are both "nothing here to damage or repair" — the browser's
 * advance reading of the refusal the apply tick would settle on.
 */
export function scopeIsDamageable(entity, scope) {
  return entityIsDamageable(entity) && !!gmEffectScopeTotals(entity, scope);
}

function entryKey(operatorId, correlation) {
  return JSON.stringify([operatorId, correlation]);
}

/** Mount the direct damage/repair panel over injected page/transport seams. */
export function createGmDirectEffectPanel({
  doc = globalThis.document,
  win = doc && doc.defaultView,
  t = (id) => id,
  displayText = wireText,
  submitDirectEffect = null,
  getOperator = () => null,
  getOperatorName = (id) => id,
  correlation = createActionCorrelation,
  now,
  capacity = GM_EFFECT_FEED_CAPACITY,
  timeoutMs = DEFAULT_ACTION_FEEDBACK_TIMEOUT_MS,
  schedule = (fn, delay) => setTimeout(fn, delay),
  cancelSchedule = (timer) => clearTimeout(timer),
  actionFeedback: suppliedActionFeedback = null,
} = {}) {
  const region = doc && doc.getElementById('gm-effect-panel');
  const empty = doc && doc.getElementById('gm-effect-empty');
  const controls = doc && doc.getElementById('gm-effect-controls');
  const targetLine = doc && doc.getElementById('gm-effect-target');
  const hullLine = doc && doc.getElementById('gm-effect-hull');
  const scopeSelect = doc && doc.getElementById('gm-effect-scope');
  const amountInput = doc && doc.getElementById('gm-effect-amount');
  const damageButton = doc && doc.getElementById('gm-effect-damage');
  const healButton = doc && doc.getElementById('gm-effect-heal');
  const warning = doc && doc.getElementById('gm-effect-warning');
  const feedbackStatus = doc && doc.getElementById('gm-effect-feedback');
  const log = doc && doc.getElementById('gm-effect-log');

  const boundedCapacity = Math.max(
    1,
    Number.isInteger(capacity) ? capacity : GM_EFFECT_FEED_CAPACITY,
  );
  const boundedTimeoutMs = Number.isFinite(timeoutMs) && timeoutMs >= 0
    ? timeoutMs : DEFAULT_ACTION_FEEDBACK_TIMEOUT_MS;

  let selected = null;
  // The scope KEY rather than the scope, so a selection that survives a
  // projection push survives it by identity: the same Station on a re-published
  // entity is the same choice, and one whose Systems have gone falls back to
  // the whole hull rather than staying pointed at nothing.
  let scopeKey = 'entity';
  // What the picker's DOM currently shows, so an unchanged list is left alone.
  let renderedScopeSignature = null;
  let authoritativeResults = [];
  const pending = new Map();
  const localTerminals = new Map();

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

  function refusalText(reason) {
    if (!reason) return t('server.gm.effect.reason_unspecified');
    if (reason === LOCAL_INGRESS_REFUSAL) {
      return t('server.gm.session.reason.ingress_rejected');
    }
    const labelId = GM_ACTION_REFUSAL_REASON_LABELS[reason];
    return labelId ? t(labelId) : t('server.gm.effect.reason_unknown', { reason });
  }

  function requestedMilliHp() {
    return milliHpFromInput(amountInput ? amountInput.value : null);
  }

  /** The chosen scope, or `null` for the whole entity (issue #1311). */
  function scope() {
    const chosen = gmEffectScopeFromKey(scopeKey);
    return chosen === undefined ? null : chosen;
  }

  /** The authored display name the projection published for `chosen`. */
  function scopeName(chosen) {
    if (!chosen) return null;
    for (const option of gmEffectScopeOptions(selected)) {
      if (option.scope && gmEffectScopeKey(option.scope) === gmEffectScopeKey(chosen)) {
        return option.name;
      }
    }
    return null;
  }

  /**
   * The localised sentence naming `chosen`.
   *
   * The name goes through `displayText` for the reason the target line's does:
   * `gm_entity` is a raw-DTO exception on the Host Channel localisation
   * boundary, so the authored display name of a Station or a System alike
   * arrives as a String Table id and is resolved at render time. Both halves of
   * the picker are named that way. A scope the projection named nothing for
   * falls back to its authoring key, which `wireText` passes through untouched
   * — the same verbatim treatment the mission panel gives an authored event id.
   */
  function scopeText(chosen) {
    const name = scopeName(chosen);
    return gmEffectScopeLabel(t, chosen, name == null ? null : displayText(name));
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
    row.className = 'gm-effect-log-entry';
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
    if (result.target) row.dataset.entity = result.target;
    if (result.reason) row.dataset.reason = result.reason;
    if (result.scope) row.dataset.scope = gmEffectScopeKey(result.scope);
    if (result.effect) {
      row.dataset.effect = result.effect.kind;
      row.dataset.applied = String(result.effect.applied_milli_hp);
      row.dataset.discarded = String(result.effect.discarded_milli_hp);
      row.dataset.destroyed = String(result.effect.destroyed);
    }
    let text = t(`server.gm.effect.result_${suffix}`, {
      name: operatorName(result.operator_id),
      entity: result.target || '',
      amount: hullPoints(result.effect ? result.effect.applied_milli_hp : 0),
      tick: String(result.tick),
      correlation: result.correlation,
      reason: refusalText(result.reason),
    });
    // The narrowing is named BEFORE the consequences, because "20 hull to
    // Courier" and "20 hull to Courier's helm" are different facts and only the
    // second is one a GM can act on.
    if (result.scope) {
      text += t(`server.gm.activity.action.direct_effect_${result.scope.kind}`, {
        scope: result.scope.id,
      });
    }
    if (result.effect && result.effect.destroyed) {
      text += t('server.gm.effect.result_destroyed');
    }
    if (result.effect && result.effect.discarded_milli_hp > 0) {
      text += t('server.gm.effect.result_discarded', {
        amount: hullPoints(result.effect.discarded_milli_hp),
      });
    }
    row.textContent = text;
  }

  function paintLocalRow(meta, outcome, reason) {
    const row = appendRow(meta.operatorId, meta.correlation);
    if (!row) return;
    row.dataset.outcome = outcome;
    row.dataset.entity = meta.entity;
    row.dataset.effect = meta.kind;
    if (reason) row.dataset.reason = reason;
    const statusId = {
      pending: 'server.gm.effect.result_pending',
      'timed-out': 'server.gm.effect.result_timed_out',
      refused: 'server.gm.effect.result_refused_local',
    }[outcome];
    row.textContent = t(statusId, {
      name: meta.operatorName,
      entity: meta.entity,
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

  function paintFeedback(state, entity) {
    if (!feedbackStatus) return;
    feedbackStatus.dataset.state = state || '';
    feedbackStatus.dataset.entity = entity || '';
    const statusId = FEEDBACK_STATUS_IDS[state];
    feedbackStatus.textContent = statusId
      ? t('action_feedback.summary', {
          action: t('server.gm.effect.heading'),
          status: t(statusId),
        })
      : '';
  }

  function paintWarning() {
    if (!warning) return;
    const amount = requestedMilliHp();
    const chosen = scope();
    const damage = previewDirectEffect(selected, 'damage', amount, chosen);
    const heal = previewDirectEffect(selected, 'heal', amount, chosen);
    if (!damage || !heal) {
      warning.textContent = '';
      delete warning.dataset.lethal;
      delete warning.dataset.emptied;
      delete warning.dataset.discarded;
      return;
    }
    let text = '';
    if (damage.destroyed) {
      warning.dataset.lethal = 'true';
      text = chosen
        ? t('server.gm.effect.lethal_warning_scoped', {
            name: displayText(selected.name),
            scope: scopeText(chosen),
          })
        : t('server.gm.effect.lethal_warning', { name: displayText(selected.name) });
    } else {
      delete warning.dataset.lethal;
    }
    // Emptying a Station without sinking the ship is its own warning, because
    // it is the one consequence a narrowed press has that a whole-hull press
    // never does: a console goes dark and the entity survives to notice.
    if (!damage.destroyed && damage.emptied && chosen) {
      warning.dataset.emptied = 'true';
      text = t('server.gm.effect.scope_emptied_warning', { scope: scopeText(chosen) });
    } else {
      delete warning.dataset.emptied;
    }
    // Damage and repair discard different remainders, so the warning names the
    // larger of the two rather than pretending one number covers both presses.
    const discarded = Math.max(damage.discarded_milli_hp, heal.discarded_milli_hp);
    if (discarded > 0) {
      warning.dataset.discarded = String(discarded);
      const overflow = t('server.gm.effect.overflow_warning', {
        amount: hullPoints(discarded),
      });
      text = text ? `${text} ${overflow}` : overflow;
    } else {
      delete warning.dataset.discarded;
    }
    warning.textContent = text;
  }

  /**
   * Rebuild the Scope picker from the projection's own per-System breakdown.
   *
   * Every option is an identity the simulation published, in hull order, and a
   * selection whose Station or System is no longer there falls back to the
   * whole entity rather than staying pointed at nothing — the same rule the
   * inspector follows when a selected entity leaves the world.
   */
  function renderScopeOptions() {
    if (!scopeSelect) return;
    const options = gmEffectScopeOptions(selected);
    const keys = options.map((option) => gmEffectScopeKey(option.scope));
    const unavailable = !keys.includes(scopeKey);
    if (unavailable) options.push({ scope: scope(), unavailable: true });
    // The projection pushes on every change to any entity, and hull totals
    // change constantly in a fight. Rebuilding an unchanged option list would
    // collapse an open dropdown under the operator mid-choice, so the DOM is
    // rewritten only when the CHOICES move — the live numbers live on the hull
    // line, which is repainted every time.
    const signature = JSON.stringify(options.map((option) => [
      gmEffectScopeKey(option.scope),
      option.name,
      option.unavailable === true,
    ]));
    if (signature === renderedScopeSignature) {
      scopeSelect.value = scopeKey;
      return;
    }
    renderedScopeSignature = signature;
    scopeSelect.replaceChildren();
    for (const option of options) {
      const key = gmEffectScopeKey(option.scope);
      const node = doc.createElement('option');
      node.value = key;
      node.textContent = gmEffectScopeLabel(
        t,
        option.scope,
        option.name == null ? null : displayText(option.name),
      );
      if (option.unavailable) {
        node.textContent = t('server.gm.effect.scope_undamageable', { scope: scopeText(option.scope) });
        node.disabled = true;
      }
      if (option.scope) node.dataset.scopeKind = option.scope.kind;
      node.selected = key === scopeKey;
      scopeSelect.appendChild(node);
    }
    scopeSelect.value = scopeKey;
    // One option means the entity publishes no breakdown at all, so there is
    // nothing to choose between and the control says so rather than offering a
    // list of one.
    scopeSelect.disabled = options.length <= 1;
  }

  function renderTarget() {
    const damageable = entityIsDamageable(selected);
    if (region) {
      region.dataset.entityId = selected ? selected.entity_id : '';
      region.dataset.damageable = String(damageable);
    }
    if (empty) {
      empty.hidden = damageable;
      empty.textContent = damageable
        ? ''
        : t(selected ? 'server.gm.effect.undamageable' : 'server.gm.effect.empty');
    }
    if (controls) controls.hidden = !damageable;
    renderScopeOptions();
    const chosen = scope();
    const totals = gmEffectScopeTotals(selected, chosen);
    if (region) region.dataset.scope = gmEffectScopeKey(chosen);
    if (targetLine) {
      targetLine.textContent = damageable
        ? t('server.gm.effect.target', { name: displayText(selected.name) })
        : '';
    }
    if (hullLine) {
      if (!damageable) {
        hullLine.textContent = '';
      } else if (!totals) {
        // The scope survived the option rebuild but names nothing damageable —
        // the browser's advance reading of the refusal the apply tick settles.
        hullLine.textContent = t('server.gm.effect.scope_undamageable', {
          scope: scopeText(chosen),
        });
      } else if (chosen) {
        hullLine.textContent = t('server.gm.effect.scope_hull', {
          scope: scopeText(chosen),
          current: hullPoints(totals.current),
          max: hullPoints(totals.max),
        });
      } else {
        hullLine.textContent = t('server.gm.effect.hull', {
          current: hullPoints(totals.current),
          max: hullPoints(totals.max),
        });
      }
    }
    paintWarning();
    refreshAdmission();
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
      meta.entity,
    );
    renderLog();
    refreshAdmission();
    return true;
  }

  /** Whether this operator already has an unsettled press on the selection. */
  function hasPendingFor(entityId) {
    for (const meta of pending.values()) {
      if (meta.entity === entityId) return true;
    }
    return false;
  }

  /** Submit one absolute direct effect against the current selection. */
  function apply(kind) {
    const current = operator();
    const amount = requestedMilliHp();
    const chosen = scope();
    // A scope naming nothing damageable is refused HERE rather than sent, for
    // the reason an empty amount is: the browser already holds the projection
    // that answers it, and a press it can see will be refused is a press that
    // should never take a journal slot.
    if (!current || !EFFECT_KINDS.has(kind) || !scopeIsDamageable(selected, chosen)
        || amount === null || hasPendingFor(selected.entity_id)) return false;
    while (pending.size >= boundedCapacity) {
      const oldest = pending.keys().next().value;
      if (oldest === undefined) break;
      finishLocalPending(oldest, 'timed-out', null);
    }
    const press = actionFeedback.press(
      `${GM_EFFECT_ACTION_PREFIX}${kind}:${selected.entity_id}:${gmEffectScopeKey(chosen)}`,
    );
    const meta = {
      entity: selected.entity_id,
      kind,
      scope: chosen,
      amountMilliHp: amount,
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
      accepted = typeof submitDirectEffect === 'function'
        && submitDirectEffect({
          entity: meta.entity,
          effect: kind,
          amount_milli_hp: amount,
          correlation: press.correlation,
          ...gmEffectScopeRequestFields(chosen),
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
    paintFeedback(ACTION_FEEDBACK_STATE.PENDING, meta.entity);
    renderLog();
    refreshAdmission();
    return true;
  }

  function refreshAdmission() {
    const admitted = !!operator();
    const amount = requestedMilliHp();
    const chosen = scope();
    const ready = admitted && scopeIsDamageable(selected, chosen) && amount !== null
      && !hasPendingFor(selected.entity_id);
    if (region) region.dataset.admitted = String(admitted);
    for (const [button, kind] of [[damageButton, 'damage'], [healButton, 'heal']]) {
      if (!button) continue;
      button.disabled = !ready;
      button.setAttribute('aria-disabled', ready ? 'false' : 'true');
      const verb = kind === 'heal' ? 'heal' : 'damage';
      // A narrowed press gets its own accessible sentence rather than the
      // whole-hull one: a screen reader that said "Damage Courier" while the
      // picker read "helm" would describe a press the button does not make.
      button.setAttribute(
        'aria-label',
        chosen
          ? t(`server.gm.effect.${verb}_scoped_accessibility`, {
              name: selected ? displayText(selected.name) : '',
              scope: scopeText(chosen),
              amount: hullPoints(amount || 0),
            })
          : t(`server.gm.effect.${verb}_accessibility`, {
              name: selected ? displayText(selected.name) : '',
              amount: hullPoints(amount || 0),
            }),
      );
    }
    return admitted;
  }

  function onDamageClick(event) {
    if (event && typeof event.preventDefault === 'function') event.preventDefault();
    apply('damage');
  }

  function onHealClick(event) {
    if (event && typeof event.preventDefault === 'function') event.preventDefault();
    apply('heal');
  }

  function onAmountInput() {
    paintWarning();
    refreshAdmission();
  }

  function onScopeChange() {
    scopeKey = scopeSelect ? scopeSelect.value : 'entity';
    // A full re-render rather than a warning repaint: the hull line, the
    // accessible names and the readiness of both buttons all answer the scope.
    renderTarget();
  }

  if (damageButton) damageButton.addEventListener('click', onDamageClick);
  if (healButton) healButton.addEventListener('click', onHealClick);
  if (amountInput) amountInput.addEventListener('input', onAmountInput);
  if (scopeSelect) scopeSelect.addEventListener('change', onScopeChange);

  /** Absolute selection push from the map/inspector projection. */
  function select(entity) {
    selected = entity && typeof entity === 'object' ? entity : null;
    renderTarget();
    return !!selected;
  }

  /** Fold one absolute `gm_entity` payload's attributed result feed. */
  function update(payload) {
    const results = parseGmEffectResults(payload);
    if (results === undefined) return false;
    authoritativeResults = results.slice(-boundedCapacity);
    localTerminals.clear();
    for (const result of results) {
      const meta = pending.get(result.correlation);
      if (!meta || meta.operatorId !== result.operator_id) continue;
      clearPendingTimer(meta);
      pending.delete(result.correlation);
      const state = result.outcome === 'refused'
        ? ACTION_FEEDBACK_STATE.REFUSED
        : ACTION_FEEDBACK_STATE.APPLIED;
      actionFeedback.settle(result.correlation, state);
      paintFeedback(state, meta.entity);
    }
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
    authoritativeResults = [];
    selected = null;
    scopeKey = 'entity';
    renderedScopeSignature = null;
    if (log) log.replaceChildren();
    paintFeedback(null, null);
    renderTarget();
  }

  function destroy() {
    if (damageButton) damageButton.removeEventListener('click', onDamageClick);
    if (healButton) healButton.removeEventListener('click', onHealClick);
    if (amountInput) amountInput.removeEventListener('input', onAmountInput);
    if (scopeSelect) scopeSelect.removeEventListener('change', onScopeChange);
    for (const meta of pending.values()) clearPendingTimer(meta);
    pending.clear();
  }

  renderTarget();

  return {
    actionFeedback,
    apply,
    select,
    update,
    reset,
    refreshAdmission,
    preview: (kind) => previewDirectEffect(selected, kind, requestedMilliHp(), scope()),
    /** Absolute scope push, for the same reason `select` is one. */
    selectScope: (key) => {
      const chosen = gmEffectScopeFromKey(key);
      if (chosen === undefined) return false;
      scopeKey = gmEffectScopeKey(chosen);
      renderTarget();
      return scopeKey === gmEffectScopeKey(scope());
    },
    state: () => ({
      selected: selected ? selected.entity_id : null,
      damageable: entityIsDamageable(selected),
      scope: gmEffectScopeKey(scope()),
      scopeDamageable: scopeIsDamageable(selected, scope()),
      amountMilliHp: requestedMilliHp(),
      pending: pending.size,
      authoritative: authoritativeResults.length,
    }),
    destroy,
    win,
  };
}
