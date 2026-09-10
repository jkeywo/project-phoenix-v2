/**
 * The saved GM action history (issue #1441).
 *
 * This panel READS the one canonical journal. `GmSessionProjection.journal` is
 * a public projection of `GmActionLog`, which Rust recomputes from the durable,
 * snapshot-folded `GmActionJournal`; there is no second store on this page and
 * nothing here is authoritative. A restore that rewinds the journal therefore
 * rewinds what this panel shows, and entries made after the save simply stop
 * arriving — the panel keeps no history of its own that could survive as an
 * abandoned timeline.
 *
 * Reading discipline (PRD #1418, stories 23/31): filters and the selected entry
 * survive every republish, focus is put back where it was, status is spelled out
 * in words as well as carried on `data-outcome`, and nothing here ever opens a
 * dialog — the journal is routine attention, not an interruption.
 */
import { wireText } from './strings.js';
import { createActionCorrelation, DEFAULT_ACTION_FEEDBACK_TIMEOUT_MS } from './action-feedback.js';
import { GM_ACTION_REFUSAL_REASON_LABELS } from './gm-action-reasons.js';
import {
  createGmInversePreview,
  gmAffectedFieldText,
  gmInverseAvailability,
  gmInverseSupportedKinds,
  normaliseGmSpawnExposure,
} from './gm-inverse-preview.js';

/** The T2 confirmation category an Undo selects. Registered centrally. */
export const GM_UNDO_CONFIRMATION = Object.freeze({ category: 'action.undo', defaultMode: 'confirm-preview' });

const OUTCOMES = Object.freeze(['applied', 'no-op', 'refused']);
/** Outcome ids contain a hyphen; String Table ids do not. */
const OUTCOME_LABELS = Object.freeze({
  applied: 'server.gm.journal.outcome.applied',
  'no-op': 'server.gm.journal.outcome.no_op',
  refused: 'server.gm.journal.outcome.refused',
});

/**
 * Wire `GmActionKind` → String Table id, in the same spirit as
 * `GM_ACTION_REFUSAL_REASON_LABELS`: the wire spelling stays machine-readable
 * and its copy is localised. A kind with no row here is rendered through
 * `kind_unknown`, which keeps the diagnostic identity visible instead of
 * discarding the row — `GmActionKind` is append-only, and a history surface
 * that blanked itself because a newer host logged a newer family would be worse
 * than one that says "unknown action (…)".
 */
export const GM_JOURNAL_ACTION_KIND_LABELS = Object.freeze({
  'session-pause': 'server.gm.journal.kind.session_pause',
  'station-puppet': 'server.gm.journal.kind.station_puppet',
  'station-command': 'server.gm.journal.kind.station_command',
  'event-control': 'server.gm.journal.kind.event_control',
  'direct-effect': 'server.gm.journal.kind.direct_effect',
  'world-spawn': 'server.gm.journal.kind.world_spawn',
  'world-despawn': 'server.gm.journal.kind.world_despawn',
  'objective-control': 'server.gm.journal.kind.objective_control',
  'contact-reveal': 'server.gm.journal.kind.contact_reveal',
  'contact-conceal': 'server.gm.journal.kind.contact_conceal',
  'contact-normal': 'server.gm.journal.kind.contact_normal',
  'system-disable': 'server.gm.journal.kind.system_disable',
  'system-restore': 'server.gm.journal.kind.system_restore',
  comms: 'server.gm.journal.kind.comms',
  'npc-doctrine': 'server.gm.journal.kind.npc_doctrine',
  'faction-relation': 'server.gm.journal.kind.faction_relation',
  'action-undo': 'server.gm.journal.kind.action_undo',
});

const text = (value) => typeof value === 'string' && value.length > 0;
const count = (value) => Number.isSafeInteger(value) && value >= 0;

function normaliseEntry(value) {
  if (!value || typeof value !== 'object'
      || !text(value.operator_id) || !text(value.correlation)
      || !text(value.action_kind) || !OUTCOMES.includes(value.outcome)
      || !count(value.tick)
      || (value.target !== undefined && !text(value.target))
      || (value.sequence !== undefined && !count(value.sequence))
      || (value.reason !== undefined && !text(value.reason))
      || (value.inverted !== undefined && typeof value.inverted !== 'boolean')
      || (value.affected !== undefined
        && (!value.affected || typeof value.affected !== 'object'))
      || (value.spawn_exposure !== undefined
        && !normaliseGmSpawnExposure(value.spawn_exposure))
      || (value.undo_of !== undefined && !normaliseUndoReference(value.undo_of))) return undefined;
  return {
    operator_id: value.operator_id,
    correlation: value.correlation,
    action_kind: value.action_kind,
    outcome: value.outcome,
    tick: value.tick,
    ...(value.target === undefined ? {} : { target: value.target }),
    ...(value.sequence === undefined ? {} : { sequence: value.sequence }),
    ...(value.reason === undefined ? {} : { reason: value.reason }),
    // The affected pair travels VERBATIM. This panel never rebuilds it: what it
    // sends back when a GM asks for an inverse has to be byte-for-byte what the
    // canonical journal recorded, because that comparison is exactly what
    // refuses a request built on a stale reading (issue #1442).
    ...(value.affected === undefined ? {} : { affected: value.affected }),
    // Live eligibility of a placement (issue #1443), republished as the clock
    // runs. Unlike `affected` this is NOT echoed back with the request: it is a
    // fact about the world, and the reducer reads its own copy at the apply
    // tick rather than trusting a page's reading of it.
    ...(value.spawn_exposure === undefined
      ? {}
      : { spawn_exposure: normaliseGmSpawnExposure(value.spawn_exposure) }),
    ...(value.undo_of === undefined
      ? {}
      : { undo_of: normaliseUndoReference(value.undo_of) }),
    inverted: value.inverted === true,
  };
}

/** The public identity of the original an inverse reversed. */
function normaliseUndoReference(value) {
  if (!value || typeof value !== 'object' || !text(value.operator_id)
      || !text(value.correlation) || !count(value.sequence)) return undefined;
  return {
    operator_id: value.operator_id,
    correlation: value.correlation,
    sequence: value.sequence,
  };
}

/**
 * Parse the journal out of one absolute `gm_session` payload.
 *
 * `undefined` for a payload with no `journal` at all — the native host's
 * bootstrap push is one — so an older or partial payload leaves the last good
 * history on screen rather than blanking it.
 */
export function parseGmJournalProjection(payload) {
  let value = payload;
  if (typeof value === 'string') {
    try { value = JSON.parse(value); } catch (_) { return undefined; }
  }
  const journal = value && typeof value === 'object' ? value.journal : undefined;
  if (!journal || typeof journal !== 'object'
      || !count(journal.capacity) || !count(journal.total)
      || !Array.isArray(journal.entries)
      || journal.entries.length > journal.total) return undefined;
  const entries = [];
  for (const candidate of journal.entries) {
    const entry = normaliseEntry(candidate);
    if (!entry) return undefined;
    entries.push(entry);
  }
  return { capacity: journal.capacity, total: journal.total, entries };
}

/**
 * Stable identity of one journal row across republishes.
 *
 * JSON rather than a delimiter-joined string: an operator id or a correlation
 * is operator-chosen text, and two rows must not collide because one of them
 * happened to contain the separator.
 */
export function gmJournalEntryKey(entry) {
  return JSON.stringify([
    entry.operator_id,
    entry.correlation,
    entry.tick,
    entry.sequence ?? null,
  ]);
}

/** Operator and outcome filters compose as a strict AND. */
export function filterGmJournalEntries(entries, { operator = 'all', outcome = 'all' } = {}) {
  return entries.filter((entry) => (
    (operator === 'all' || entry.operator_id === operator)
      && (outcome === 'all' || entry.outcome === outcome)
  ));
}

/**
 * Whether this build can offer an Undo control for one journal row.
 *
 * Every clause is a fact the CANONICAL journal published, not a local guess:
 * the action really applied, it recorded the affected pair an inverse needs,
 * nothing has already reversed it, and this build has a typed inverse for its
 * family. The reducer re-checks all of it at the apply tick — this only decides
 * whether to show a control at all, because a control that cannot work is worse
 * than none.
 */
export function gmJournalEntryIsUndoable(entry) {
  return !!entry && entry.outcome === 'applied' && !!entry.affected && entry.inverted !== true
    && gmInverseAvailability(entry.action_kind, entry.spawn_exposure).supported;
}

export function createGmJournalPanel({
  doc = globalThis.document,
  t = (id) => id,
  displayText = wireText,
  getOperatorName = (id) => id,
  getOperator = () => null,
  submitUndo = () => false,
  confirmAction = (request) => request.accept(),
  correlation = createActionCorrelation,
  schedule = globalThis.setTimeout,
  cancelSchedule = globalThis.clearTimeout,
  inversePreview = null,
} = {}) {
  const el = (suffix) => doc && doc.getElementById(`gm-journal-${suffix}`);
  const region = doc && doc.getElementById('gm-journal');
  const heading = el('heading');
  const operatorFilter = el('operator-filter');
  const outcomeFilter = el('outcome-filter');
  const clearButton = el('clear-filters');
  const status = el('status');
  const empty = el('empty');
  const list = el('list');
  const detail = el('detail');
  const summary = el('detail-summary');
  const targetLine = el('detail-target');
  const outcomeLine = el('detail-outcome');
  const inverseHost = el('inverse');
  const inverseSupport = el('inverse-support');
  const undoButton = el('undo');
  const undoFeedback = el('undo-feedback');
  const inverse = inversePreview || createGmInversePreview({ doc, t, displayText });

  let state = { capacity: 0, total: 0, entries: [] };
  let selectedKey = null;
  let pending = null;
  let timer = null;

  if (region) {
    region.setAttribute('role', 'region');
    if (heading) region.setAttribute('aria-labelledby', heading.id);
  }
  if (status) {
    status.setAttribute('role', 'status');
    status.setAttribute('aria-live', 'polite');
    status.setAttribute('aria-atomic', 'true');
  }
  if (list) list.setAttribute('role', 'list');
  if (empty) empty.textContent = t('server.gm.journal.empty');
  // Panel-level answer to "what can I undo?", so a GM does not have to open
  // entries one by one to discover that nothing in this build is reversible.
  // Derived from the shared table, never asserted here.
  if (inverseSupport) {
    const supported = gmInverseSupportedKinds();
    inverseSupport.textContent = supported.length === 0
      ? t('server.gm.journal.inverse_support_none')
      : t('server.gm.journal.inverse_support_some', {
        kinds: supported
          .map((kind) => (GM_JOURNAL_ACTION_KIND_LABELS[kind]
            ? t(GM_JOURNAL_ACTION_KIND_LABELS[kind])
            : kind))
          .join(', '),
      });
  }
  if (clearButton) clearButton.textContent = t('server.gm.journal.filter.clear');
  if (undoButton) undoButton.textContent = t('server.gm.journal.undo');
  if (undoFeedback) {
    undoFeedback.setAttribute('role', 'status');
    undoFeedback.setAttribute('aria-live', 'polite');
  }
  if (outcomeFilter) {
    for (const option of outcomeFilter.options) {
      option.textContent = t(option.value === 'all'
        ? 'server.gm.journal.filter.outcome_all'
        : OUTCOME_LABELS[option.value] || 'server.gm.journal.filter.outcome_all');
    }
  }

  const operatorName = (id) => {
    let name = id;
    try { name = getOperatorName(id) || id; } catch (_) { name = id; }
    return displayText(name, name);
  };
  const kindLabel = (entry) => (GM_JOURNAL_ACTION_KIND_LABELS[entry.action_kind]
    ? t(GM_JOURNAL_ACTION_KIND_LABELS[entry.action_kind])
    : t('server.gm.journal.kind_unknown', { kind: entry.action_kind }));
  const actionText = (entry) => (entry.target
    ? t('server.gm.journal.action_target', {
      action: kindLabel(entry),
      target: displayText(entry.target, entry.target),
    })
    : kindLabel(entry));
  const orderText = (entry) => (entry.sequence === undefined
    ? t('server.gm.journal.order_unsequenced', { tick: String(entry.tick) })
    : t('server.gm.journal.order', {
      tick: String(entry.tick),
      sequence: String(entry.sequence),
    }));
  const outcomeText = (entry) => t(OUTCOME_LABELS[entry.outcome]);
  const reasonText = (entry) => (entry.reason
    ? t(GM_ACTION_REFUSAL_REASON_LABELS[entry.reason] || 'server.gm.effect.reason_unknown', {
      reason: entry.reason,
    })
    : '');

  const selected = () => state.entries.find((entry) => gmJournalEntryKey(entry) === selectedKey);

  // Words, never colour alone, and never a dialog: an inverse that is still
  // waiting for its canonical answer is routine attention (#1418 stories 6/31).
  function undoFeedbackState(value) {
    if (!undoFeedback) return;
    undoFeedback.dataset.state = value || '';
    undoFeedback.textContent = value ? t(`server.gm.journal.undo_${value}`) : '';
  }

  function clearPending() {
    if (timer !== null) cancelSchedule(timer);
    timer = null;
    pending = null;
  }

  /**
   * Ask the canonical reducer to reverse the selected entry.
   *
   * The request carries the original's PUBLIC identity and the recorded pair
   * verbatim. Nothing here decides the outcome: a target that moved, a second
   * GM who got there first and a stale reading are all answered at the apply
   * tick, and this surface only reports what came back.
   */
  function requestUndo() {
    const entry = selected();
    const operator = getOperator();
    if (!entry || !operator || pending || !gmJournalEntryIsUndoable(entry)) return false;
    const described = gmAffectedFieldText(entry.affected, t, displayText);
    const description = t('server.gm.journal.undo_confirm', {
      operator: operatorName(entry.operator_id),
      action: actionText(entry),
      order: orderText(entry),
      change: described
        ? t('server.gm.journal.undo_change', { before: described.after, after: described.before })
        : t('server.gm.inverse.state_uncaptured'),
    });
    const captured = Object.freeze({
      action: 'undo_gm_action',
      operator_id: operator.id,
      original: entry.correlation,
      original_operator: entry.operator_id,
      original_sequence: entry.sequence,
      expected: entry.affected,
    });
    let consumed = false;
    return confirmAction({
      ...GM_UNDO_CONFIRMATION,
      intent: captured,
      description,
      // The consequence sentences a GM must be able to read even when their own
      // policy skips the confirmation step: the preview repeats them, and the
      // detail region below shows the same text with no dialog at all.
      preview: () => `${description} ${t('server.gm.inverse.witnessed_note')}`,
      onCancel() { consumed = true; },
      accept() {
        if (consumed) return false;
        consumed = true;
        if (pending || getOperator()?.id !== captured.operator_id) return false;
        const request = { ...captured, correlation: correlation() };
        let accepted = false;
        try { accepted = submitUndo(request) !== false; } catch (_) { accepted = false; }
        if (!accepted) {
          undoFeedbackState('refused');
          return false;
        }
        pending = request;
        undoFeedbackState('pending');
        paintDetail();
        timer = schedule(() => {
          timer = null;
          pending = null;
          undoFeedbackState('timed_out');
          paintDetail();
        }, DEFAULT_ACTION_FEEDBACK_TIMEOUT_MS);
        return true;
      },
    }) !== false;
  }

  function paintDetail() {
    const entry = selected();
    if (detail) detail.hidden = !entry;
    if (!entry) {
      inverse.clear(inverseHost);
      if (undoButton) undoButton.hidden = true;
      for (const node of [summary, targetLine, outcomeLine]) if (node) node.textContent = '';
      return;
    }
    if (summary) {
      summary.textContent = t('server.gm.journal.detail_summary', {
        operator: operatorName(entry.operator_id),
        action: actionText(entry),
        order: orderText(entry),
      });
    }
    if (targetLine) {
      targetLine.textContent = entry.target
        ? t('server.gm.journal.detail_target', { target: displayText(entry.target, entry.target) })
        : t('server.gm.journal.detail_no_target');
    }
    if (outcomeLine) {
      outcomeLine.dataset.outcome = entry.outcome;
      outcomeLine.textContent = entry.reason
        ? t('server.gm.journal.detail_outcome_reason', {
          outcome: outcomeText(entry),
          reason: reasonText(entry),
        })
        : t('server.gm.journal.detail_outcome', { outcome: outcomeText(entry) });
    }
    // The inverse story, from the shared component: the before/after pair the
    // canonical journal recorded, what the action technically did, what crews
    // already witnessed, and whether an inverse exists.
    inverse.render(inverseHost, {
      actionKind: entry.action_kind,
      target: entry.target,
      affected: entry.affected,
      exposure: entry.spawn_exposure,
      technical: entry.outcome === 'applied'
        ? t('server.gm.inverse.technical_applied', { tick: String(entry.tick) })
        : t(entry.outcome === 'no-op'
          ? 'server.gm.inverse.technical_no_op'
          : 'server.gm.inverse.technical_refused'),
    });
    if (undoButton) {
      // Hidden rather than disabled when no inverse can run: the eligibility
      // sentence above already says why, and a dead control in a live event is
      // worse than no control (PRD #1418 story 29).
      const offerable = gmJournalEntryIsUndoable(entry) && !!getOperator();
      undoButton.hidden = !offerable;
      undoButton.disabled = !offerable || !!pending;
      undoButton.setAttribute('aria-label', t('server.gm.journal.undo_entry', {
        action: actionText(entry),
        operator: operatorName(entry.operator_id),
        order: orderText(entry),
      }));
    }
    if (entry.inverted && undoFeedback && !pending && !undoFeedback.dataset.state) {
      undoFeedback.textContent = t('server.gm.journal.undo_already');
    }
  }

  function paintSelection() {
    if (!list) return;
    for (const button of list.querySelectorAll('.gm-journal-row')) {
      const isSelected = button.dataset.key === selectedKey;
      button.setAttribute('aria-pressed', String(isSelected));
      button.dataset.selected = String(isSelected);
    }
  }

  function select(key) {
    const next = state.entries.some((entry) => gmJournalEntryKey(entry) === key) ? key : null;
    if (next === selectedKey) return next !== null;
    selectedKey = next;
    paintSelection();
    paintDetail();
    return next !== null;
  }

  function moveFocus(from, step) {
    const buttons = [...list.querySelectorAll('.gm-journal-row')];
    const index = buttons.indexOf(from);
    if (index < 0) return;
    const next = step === 'home' ? 0
      : step === 'end' ? buttons.length - 1
        : Math.min(buttons.length - 1, Math.max(0, index + step));
    buttons[next]?.focus();
  }

  function rowButton(entry) {
    const key = gmJournalEntryKey(entry);
    const button = doc.createElement('button');
    button.type = 'button';
    button.className = 'gm-journal-row';
    button.dataset.key = key;
    button.dataset.outcome = entry.outcome;
    button.dataset.correlation = entry.correlation;
    button.setAttribute('aria-label', t('server.gm.journal.select_entry', {
      operator: operatorName(entry.operator_id),
      action: actionText(entry),
      order: orderText(entry),
      outcome: outcomeText(entry),
    }));
    for (const [className, value] of [
      ['gm-journal-order', orderText(entry)],
      ['gm-journal-operator', operatorName(entry.operator_id)],
      ['gm-journal-action', actionText(entry)],
      // Words, not only `data-outcome`: the status has to survive a contrast
      // or forced-colour mode that flattens the row's styling (#1418 story 6).
      ['gm-journal-outcome', outcomeText(entry)],
    ]) {
      const span = doc.createElement('span');
      span.className = className;
      span.textContent = value;
      button.appendChild(span);
    }
    button.addEventListener('click', () => select(key));
    button.addEventListener('keydown', (event) => {
      const step = event.key === 'ArrowDown' ? 1
        : event.key === 'ArrowUp' ? -1
          : event.key === 'Home' ? 'home'
            : event.key === 'End' ? 'end' : null;
      if (step === null) return;
      event.preventDefault();
      moveFocus(button, step);
    });
    return button;
  }

  function rebuildOperatorFilter() {
    if (!operatorFilter) return;
    const wanted = operatorFilter.value || 'all';
    const operators = [...new Set(state.entries.map((entry) => entry.operator_id))].sort();
    operatorFilter.replaceChildren();
    const all = doc.createElement('option');
    all.value = 'all';
    all.textContent = t('server.gm.journal.filter.operator_all');
    operatorFilter.appendChild(all);
    for (const id of operators) {
      const option = doc.createElement('option');
      option.value = id;
      option.textContent = operatorName(id);
      operatorFilter.appendChild(option);
    }
    // A filter on an operator who has not left the journal survives; one whose
    // rows are gone falls back rather than hiding everything silently.
    operatorFilter.value = operators.includes(wanted) ? wanted : 'all';
  }

  function render({ rebuildOperators = true } = {}) {
    if (rebuildOperators) rebuildOperatorFilter();
    const filtered = filterGmJournalEntries(state.entries, {
      operator: operatorFilter?.value || 'all',
      outcome: outcomeFilter?.value || 'all',
    });
    if (list) {
      // Reading position and focus belong to the operator, not to the feed.
      const active = doc.activeElement;
      const focusedKey = active && list.contains(active) ? active.dataset.key : null;
      list.replaceChildren();
      for (const entry of filtered) {
        const item = doc.createElement('li');
        item.className = 'gm-journal-entry';
        item.appendChild(rowButton(entry));
        list.appendChild(item);
      }
      if (focusedKey) {
        [...list.querySelectorAll('.gm-journal-row')]
          .find((button) => button.dataset.key === focusedKey)?.focus();
      }
    }
    if (empty) empty.hidden = filtered.length !== 0;
    if (status) {
      status.textContent = t('server.gm.journal.status', {
        shown: String(filtered.length),
        total: String(state.total),
        capacity: String(state.capacity),
      });
    }
    // Selection is validated against the whole journal, not the filtered view:
    // narrowing the list must not silently drop the entry being read.
    if (selectedKey && !selected()) selectedKey = null;
    paintSelection();
    paintDetail();
  }

  function update(payload) {
    const next = parseGmJournalProjection(payload);
    if (!next) return false;
    state = next;
    // The canonical answer arrives as an ordinary journal row, because an
    // inverse IS an ordinary journal entry. No second result feed.
    const terminal = pending && state.entries.find((entry) => (
      entry.operator_id === pending.operator_id && entry.correlation === pending.correlation
    ));
    if (terminal) {
      clearPending();
      undoFeedbackState(terminal.outcome === 'applied' ? 'applied'
        : terminal.outcome === 'no-op' ? 'no_op' : 'refused');
    }
    render();
    return true;
  }

  function clearFilters() {
    if (operatorFilter) operatorFilter.value = 'all';
    if (outcomeFilter) outcomeFilter.value = 'all';
    render({ rebuildOperators: false });
  }

  function reset() {
    clearPending();
    undoFeedbackState('');
    state = { capacity: 0, total: 0, entries: [] };
    selectedKey = null;
    if (operatorFilter) operatorFilter.value = 'all';
    if (outcomeFilter) outcomeFilter.value = 'all';
    render();
  }

  const onFilter = () => render({ rebuildOperators: false });
  operatorFilter?.addEventListener('change', onFilter);
  outcomeFilter?.addEventListener('change', onFilter);
  clearButton?.addEventListener('click', clearFilters);
  undoButton?.addEventListener('click', requestUndo);
  render();

  return {
    update,
    select,
    clearFilters,
    reset,
    requestUndo,
    refreshAdmission: paintDetail,
    inverseAvailability: () => inverse.state(),
    state: () => ({
      capacity: state.capacity,
      total: state.total,
      entries: state.entries.map((entry) => ({ ...entry })),
      selected: selected() ? { ...selected() } : null,
    }),
    pending: () => (pending ? { ...pending } : null),
    destroy: () => {
      clearPending();
      operatorFilter?.removeEventListener('change', onFilter);
      outcomeFilter?.removeEventListener('change', onFilter);
      clearButton?.removeEventListener('click', clearFilters);
      undoButton?.removeEventListener('click', requestUndo);
    },
  };
}
