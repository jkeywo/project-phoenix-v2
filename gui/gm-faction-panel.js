/**
 * The GM faction-relation control (issue #1442).
 *
 * It is a narrow adapter, not a faction editor. The only two identities it can
 * name are authored factions the running world actually loaded — the list comes
 * from `GmSessionProjection.factions`, which projects the live
 * `FactionRegistry` — and the only thing it can say about a pair is whether the
 * first treats the second as hostile. There is no free-text field, no UUID, and
 * no way to invent a faction, because the typed `SetFactionHostility` action it
 * submits resolves both names against that same registry at the apply tick and
 * refuses anything else.
 *
 * Hostility is asymmetric by construction, exactly as
 * `crate::ai::faction::is_enemy` documents, so the two selects are labelled as
 * an ordered pair and the readout names the direction rather than implying a
 * mutual relationship.
 *
 * Nothing here is authoritative: the request is a proposal, the canonical
 * reducer decides, and the answer arrives as an ordinary row of the ONE saved
 * journal — which is also what makes the change undoable.
 */
import { createActionCorrelation, DEFAULT_ACTION_FEEDBACK_TIMEOUT_MS } from './action-feedback.js';
import { wireText } from './strings.js';
import { GM_ACTION_REFUSAL_REASON_LABELS } from './gm-action-reasons.js';

/** The T2 confirmation category this control selects. Registered centrally. */
export const GM_FACTION_CONFIRMATION = Object.freeze({
  category: 'faction.relation',
  defaultMode: 'confirm',
});

const text = (value) => typeof value === 'string' && value.length > 0;

/**
 * Parse the faction roster out of one absolute `gm_session` payload.
 *
 * `undefined` for a payload carrying no `factions` at all — the native host's
 * bootstrap push is one — so a partial payload leaves the last good roster on
 * screen rather than blanking the control mid-event.
 */
export function parseGmFactionRoster(payload) {
  let value = payload;
  if (typeof value === 'string') {
    try { value = JSON.parse(value); } catch (_) { return undefined; }
  }
  const rows = value && typeof value === 'object' ? value.factions : undefined;
  if (!Array.isArray(rows)) return undefined;
  const factions = [];
  for (const row of rows) {
    if (!row || typeof row !== 'object' || !text(row.name)
        || (row.label !== undefined && !text(row.label))
        || !Array.isArray(row.enemies) || row.enemies.some((id) => !text(id))) return undefined;
    factions.push({
      name: row.name,
      ...(row.label === undefined ? {} : { label: row.label }),
      enemies: [...row.enemies],
    });
  }
  if (new Set(factions.map((row) => row.name)).size !== factions.length) return undefined;
  return factions;
}

export function createGmFactionPanel({
  doc = globalThis.document,
  t = (id) => id,
  displayText = wireText,
  getOperator = () => null,
  submit = () => false,
  confirmAction = (request) => request.accept(),
  correlation = createActionCorrelation,
  schedule = globalThis.setTimeout,
  cancelSchedule = globalThis.clearTimeout,
} = {}) {
  const el = (suffix) => doc && doc.getElementById(`gm-faction-${suffix}`);
  const sourceSelect = el('source');
  const enemySelect = el('enemy');
  const relationLine = el('relation');
  const applyButton = el('apply');
  const feedback = el('feedback');
  const empty = el('empty');

  let factions = [];
  let pending = null;
  let timer = null;

  if (feedback) {
    feedback.setAttribute('role', 'status');
    feedback.setAttribute('aria-live', 'polite');
  }
  if (empty) empty.textContent = t('server.gm.faction.empty');

  const label = (name) => {
    const row = factions.find((entry) => entry.name === name);
    return row?.label ? displayText(row.label, row.name) : displayText(name, name);
  };
  const source = () => sourceSelect?.value || '';
  const enemy = () => enemySelect?.value || '';
  const row = (name) => factions.find((entry) => entry.name === name);
  const hostile = () => !!row(source())?.enemies.includes(enemy());
  const valid = () => !!getOperator() && !pending && !!row(source()) && !!row(enemy())
    && source() !== enemy();

  function feedbackState(value, detail) {
    if (!feedback) return;
    feedback.dataset.state = value || '';
    feedback.textContent = value ? t(`server.gm.faction.${value}`, detail || {}) : '';
  }

  function options(select, keep) {
    if (!select) return;
    const wanted = keep || select.value;
    select.replaceChildren();
    for (const entry of factions) {
      const option = doc.createElement('option');
      option.value = entry.name;
      option.textContent = label(entry.name);
      select.appendChild(option);
    }
    // A selection whose faction is still loaded survives every republish; one
    // whose faction has gone falls back rather than leaving a dead value.
    select.value = factions.some((entry) => entry.name === wanted)
      ? wanted
      : (factions[0]?.name || '');
  }

  function render() {
    if (empty) empty.hidden = factions.length >= 2;
    if (relationLine) {
      relationLine.textContent = valid() || (row(source()) && row(enemy()))
        ? t(hostile() ? 'server.gm.faction.is_hostile' : 'server.gm.faction.is_neutral', {
          faction: label(source()),
          enemy: label(enemy()),
        })
        : t('server.gm.faction.select');
      relationLine.dataset.hostile = String(hostile());
    }
    if (applyButton) {
      // The label states the state the press would produce, not a toggle verb:
      // the action itself is absolute, and two GMs pressing at once must not
      // have to reason about arrival order.
      applyButton.textContent = t(hostile()
        ? 'server.gm.faction.make_neutral'
        : 'server.gm.faction.make_hostile');
      applyButton.disabled = !valid();
    }
    for (const select of [sourceSelect, enemySelect]) {
      if (select) select.disabled = !!pending || factions.length === 0;
    }
  }

  function clearPending() {
    if (timer !== null) cancelSchedule(timer);
    timer = null;
    pending = null;
  }

  function apply() {
    if (!valid()) return false;
    const operator = getOperator();
    const wanted = !hostile();
    const captured = Object.freeze({
      action: 'set_faction_hostility',
      operator_id: operator.id,
      faction: source(),
      enemy: enemy(),
      hostile: wanted,
    });
    const description = t(wanted
      ? 'server.gm.faction.confirm_hostile'
      : 'server.gm.faction.confirm_neutral', {
      faction: label(captured.faction),
      enemy: label(captured.enemy),
    });
    let consumed = false;
    return confirmAction({
      ...GM_FACTION_CONFIRMATION,
      intent: captured,
      description,
      preview: () => description,
      onCancel() { consumed = true; },
      accept() {
        if (consumed) return false;
        consumed = true;
        if (pending || getOperator()?.id !== captured.operator_id) return false;
        // The relation may have moved while the dialog was open. Submit the
        // captured intent anyway so the canonical journal records the real
        // answer rather than this page silently rewriting the request.
        const request = { ...captured, correlation: correlation() };
        let accepted = false;
        try { accepted = submit(request) !== false; } catch (_) { accepted = false; }
        if (!accepted) {
          feedbackState('refused', { reason: t('server.gm.faction.reason_local') });
          return false;
        }
        pending = request;
        feedbackState('pending');
        render();
        timer = schedule(() => {
          timer = null;
          pending = null;
          feedbackState('timed_out');
          render();
        }, DEFAULT_ACTION_FEEDBACK_TIMEOUT_MS);
        return true;
      },
    }) !== false;
  }

  function update(payload) {
    const next = parseGmFactionRoster(payload);
    if (!next) return false;
    factions = next;
    options(sourceSelect);
    options(enemySelect, enemySelect?.value || factions[1]?.name);
    // The terminal answer is an ordinary row of the ONE canonical journal;
    // this panel keeps no result feed of its own.
    let value = payload;
    if (typeof value === 'string') {
      try { value = JSON.parse(value); } catch (_) { value = null; }
    }
    const entries = value?.journal?.entries;
    const terminal = pending && Array.isArray(entries) && entries.find((entry) => (
      entry && entry.operator_id === pending.operator_id
        && entry.correlation === pending.correlation
    ));
    if (terminal) {
      clearPending();
      feedbackState(terminal.outcome === 'no-op' ? 'no_op' : terminal.outcome, {
        reason: terminal.reason
          ? t(GM_ACTION_REFUSAL_REASON_LABELS[terminal.reason]
            || 'server.gm.effect.reason_unknown', { reason: terminal.reason })
          : '',
      });
    }
    render();
    return true;
  }

  function reset() {
    clearPending();
    factions = [];
    options(sourceSelect);
    options(enemySelect);
    feedbackState('');
    render();
  }

  const onChange = () => render();
  sourceSelect?.addEventListener('change', onChange);
  enemySelect?.addEventListener('change', onChange);
  applyButton?.addEventListener('click', apply);
  render();

  return {
    apply,
    update,
    reset,
    refreshAdmission: render,
    state: () => ({
      factions: factions.map((entry) => ({ ...entry, enemies: [...entry.enemies] })),
      faction: source(),
      enemy: enemy(),
      hostile: hostile(),
      pending: pending ? { ...pending } : null,
    }),
    destroy: () => {
      clearPending();
      sourceSelect?.removeEventListener('change', onChange);
      enemySelect?.removeEventListener('change', onChange);
      applyButton?.removeEventListener('click', apply);
    },
  };
}
