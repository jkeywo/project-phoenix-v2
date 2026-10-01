import { GmActionFeedback } from './gm-action-feedback.js';
import { ActionFeedbackLifecycle, createActionCorrelation, DEFAULT_ACTION_FEEDBACK_TIMEOUT_MS } from './action-feedback.js';
import { wireText } from './strings.js';
import { parseGmEffectScope } from './gm-effect-scope.js';

export const GM_SYSTEM_CONFIRMATION = Object.freeze({
  disable: Object.freeze({ category: 'system.disable', defaultMode: 'confirm' }),
  restore: Object.freeze({ category: 'system.restore', defaultMode: 'immediate' }),
});

/** Absolute System availability controls; confirmation is injected by the host. */
export function createGmSystemPanel({ doc = globalThis.document, t = id => id,
  getOperator = () => null, submit = () => false, confirmAction = request => request.accept(),
  correlation = createActionCorrelation, schedule = globalThis.setTimeout,
  cancelSchedule = globalThis.clearTimeout } = {}) {
  const el = key => doc?.getElementById(`gm-system-${key}`);
  let selected = null, system = '', controls = {}, entities = [], generation = 0;
  const actionFeedback = new ActionFeedbackLifecycle({ correlation });
  const feed = new GmActionFeedback({
    lifecycle: () => actionFeedback, capacity: 1,
    timeoutMs: DEFAULT_ACTION_FEEDBACK_TIMEOUT_MS, schedule, cancelSchedule,
    onLocalTerminal: (_meta, outcome) => { feedback(outcome === 'timed-out' ? 'timed_out' : outcome); render(); },
  });
  const pending = () => feed.values().next().value?.request ?? null;
  const current = () => controls[selected?.entity_id]?.find(row => row.system_id === system);
  const valid = () => !!getOperator() && !pending() && !!current()
    && entities.some(row => row.entity_id === selected?.entity_id);
  function feedback(state, reason = '') {
    if (el('feedback')) {
      el('feedback').dataset.state = state;
      el('feedback').textContent = t(`server.gm.system.${state}`, { reason });
    }
  }
  function render() {
    if (el('target')) el('target').textContent = selected ? wireText(selected.name) : t('server.gm.system.select');
    const row = current();
    if (el('state')) el('state').textContent = row ? t(row.gm_disabled ? 'server.gm.system.disabled' : row.available ? 'server.gm.system.available' : 'server.gm.system.offline') : '';
    for (const verb of ['disable', 'restore']) if (el(verb)) el(verb).disabled = !valid();
  }
  function options() {
    const select = el('select'); if (!select) return;
    select.replaceChildren();
    const placeholder = doc.createElement('option'); placeholder.value = ''; placeholder.textContent = t('server.gm.system.choose'); select.appendChild(placeholder);
    for (const row of controls[selected?.entity_id] || []) {
      const option = doc.createElement('option'); option.value = row.system_id; option.textContent = wireText(row.name); select.appendChild(option);
    }
    if (!current()) system = '';
    select.value = system;
  }
  function choose(disabled) {
    if (typeof disabled !== 'boolean' || !valid()) return false;
    const identity = { operator_id: getOperator().id, target: selected.entity_id, system, disabled };
    const epoch = generation;
    const verb = disabled ? 'disable' : 'restore';
    const description = t('server.gm.system.confirm', { verb: t(`server.gm.system.${verb}`), target: wireText(selected.name), system: wireText(current().name) });
    return confirmAction({ ...GM_SYSTEM_CONFIRMATION[verb], description,
      preview: () => t('server.gm.system.explanation'),
      accept: () => {
        if (epoch !== generation || pending() || getOperator()?.id !== identity.operator_id) return false;
        // The target may disappear while this private confirmation is open.
        // Submit the captured identity so ordinary Admission records its refusal.
        const request = { ...identity, correlation: actionFeedback.press(`gm.system.${verb}`).correlation };
        const meta = { request, operatorId: request.operator_id, correlation: request.correlation };
        feed.track(meta);
        actionFeedback.pending(request.correlation);
        let accepted = false;
        try { accepted = submit(request) !== false; } catch (_) { /* local refusal */ }
        feed.submitted(meta, accepted);
        if (!accepted) return false;
        if (feed.get(request.correlation) === meta) feedback('pending');
        render();
        return true;
      },
    }) !== false;
  }
  function update(payload) {
    let value = payload;
    if (typeof value === 'string') { try { value = JSON.parse(value); } catch (_) { return false; } }
    if (!value || !Array.isArray(value.entities)) return false;
    const next = value.system_controls || {}, rows = value.system_results || [];
    if (!next || typeof next !== 'object' || Array.isArray(next) || !Array.isArray(rows)
      || Object.values(next).some(list => !Array.isArray(list) || list.some(row => !row || typeof row.system_id !== 'string' || !row.system_id || typeof row.name !== 'string' || typeof row.gm_disabled !== 'boolean' || typeof row.available !== 'boolean'))
      || rows.some(row => !row || !['system-disable', 'system-restore'].includes(row.action_kind) || typeof row.target !== 'string'
        || parseGmEffectScope(row.effect_scope)?.kind !== 'system' || typeof row.operator_id !== 'string' || typeof row.correlation !== 'string'
        || !['applied', 'no-op', 'refused'].includes(row.outcome))) return false;
    entities = value.entities; controls = next;
    if (selected) selected = entities.find(row => row.entity_id === selected.entity_id) || null;
    const request = pending();
    const terminal = request && rows.find(row => row.operator_id === request.operator_id && row.correlation === request.correlation
      && row.target === request.target && row.effect_scope.system === request.system && row.action_kind === (request.disabled ? 'system-disable' : 'system-restore'));
    if (terminal) {
      feed.settle(terminal);
      feedback(terminal.outcome === 'refused' ? 'refused' : terminal.outcome === 'no-op' ? 'no_op' : 'applied');
    }
    options(); render(); return true;
  }
  function reset() {
    generation++;
    feed.reset(); selected = null; system = ''; controls = {}; entities = [];
    if (el('feedback')) el('feedback').textContent = '';
    options(); render();
  }
  el('select')?.addEventListener('change', () => { system = el('select').value; render(); });
  el('disable')?.addEventListener('click', () => choose(true));
  el('restore')?.addEventListener('click', () => choose(false));
  render();
  return { update, choose, reset, refreshAdmission: render, select(entity) { if (selected?.entity_id !== entity?.entity_id) system = ''; selected = entity; options(); render(); },
    focusSystem(systemId) {
      if (typeof systemId !== 'string' || !controls[selected?.entity_id]?.some(row => row.system_id === systemId)) return false;
      system = systemId; options(); render();
      const target = current()?.gm_disabled ? el('restore') : el('disable');
      target?.focus?.(); return doc?.activeElement === target;
    },
    state: () => ({ target: selected?.entity_id || null, system, pending: pending(), controls }) };
}
