import { createActionCorrelation, DEFAULT_ACTION_FEEDBACK_TIMEOUT_MS } from './action-feedback.js';
import { wireText } from './strings.js';
import { GM_ACTION_REFUSAL_REASON_LABELS } from './gm-action-reasons.js';

export const GM_NPC_CONFIRMATION = Object.freeze({ category: 'npc.directive', defaultMode: 'immediate' });
const bounded = value => typeof value === 'string' && value.length > 0 && new TextEncoder().encode(value).length <= 128 && !/[\u0000-\u001f\u007f-\u009f]/.test(value);

export function parseNpcDoctrinePayload(payload) {
  let value = payload;
  if (typeof value === 'string') { try { value = JSON.parse(value); } catch (_) { return null; } }
  if (!value || !Array.isArray(value.entities)) return null;
  const profiles = value.npc_doctrines || {}, results = value.npc_doctrine_results || [];
  if (typeof profiles !== 'object' || Array.isArray(profiles) || !Array.isArray(results)
    || Object.entries(profiles).some(([id, row]) => !bounded(id) || !row
      || !(row.current === null || bounded(row.current)) || !(row.intent === null || typeof row.intent === 'string')
      || !Array.isArray(row.choices) || row.choices.some(choice => !choice || !bounded(choice.id) || typeof choice.label !== 'string')
      || new Set(row.choices.map(choice => choice.id)).size !== row.choices.length)
    || results.some(row => !row || row.action_kind !== 'npc-doctrine' || !bounded(row.target) || !bounded(row.npc_doctrine)
      || !bounded(row.operator_id) || !bounded(row.correlation) || !Number.isSafeInteger(row.tick) || row.tick < 0
      || !['applied', 'no-op', 'refused'].includes(row.outcome) || !(row.reason == null || typeof row.reason === 'string'))) return null;
  return { entities: value.entities, profiles, results };
}

/** Choices and observed intent come from the ordinary peer-local projection. */
export function createGmNpcPanel({ doc = globalThis.document, t = id => id, getOperator = () => null,
  submit = () => false, confirmAction = request => request.accept(), correlation = createActionCorrelation,
  schedule = globalThis.setTimeout, cancelSchedule = globalThis.clearTimeout } = {}) {
  const el = suffix => doc?.getElementById(`gm-npc-${suffix}`);
  let selected = null, choice = '', profiles = {}, entities = [], results = [], pending = null, timer = null, generation = 0;
  const current = () => profiles[selected?.entity_id];
  const valid = () => !!getOperator() && !pending && entities.some(row => row.entity_id === selected?.entity_id)
    && !!current()?.choices.some(row => row.id === choice);
  function feedback(state) {
    if (el('feedback')) { el('feedback').dataset.state = state; el('feedback').textContent = t(`server.gm.npc.${state}`); }
  }
  function render() {
    if (el('target')) el('target').textContent = selected ? wireText(selected.name) : t('server.gm.npc.select');
    if (el('current')) el('current').textContent = current()?.current || t('server.gm.npc.original');
    if (el('intent')) el('intent').textContent = current()?.intent ? wireText(current().intent) : t('server.gm.npc.no_intent');
    if (el('apply')) el('apply').disabled = !valid();
    if (el('empty')) el('empty').hidden = !!current()?.choices.length;
  }
  function options() {
    const choices = current()?.choices || [];
    if (!choice) choice = choices.find(row => row.id === current()?.current)?.id || choices[0]?.id || '';
    if (el('choice')) {
      el('choice').replaceChildren();
      if (choice && !choices.some(row => row.id === choice)) {
        const stale = doc.createElement('option'); stale.value = choice; stale.disabled = true;
        stale.textContent = t('server.gm.npc.withdrawn', { doctrine: choice }); el('choice').appendChild(stale);
      }
      for (const item of choices) { const option = doc.createElement('option'); option.value = item.id; option.textContent = wireText(item.label); el('choice').appendChild(option); }
      el('choice').value = choice;
      el('choice').disabled = !choices.length || !!pending;
    }
  }
  function choose() {
    if (!valid()) return false;
    const captured = Object.freeze({ action: 'set_npc_doctrine', operator_id: getOperator().id, target: selected.entity_id, doctrine: choice });
    const description = t('server.gm.npc.confirm', { target: wireText(selected.name),
      doctrine: wireText(current().choices.find(row => row.id === choice).label) });
    const epoch = generation;
    let consumed = false;
    return confirmAction({ ...GM_NPC_CONFIRMATION, intent: captured, description, preview: () => description,
      onCancel() { consumed = true; }, accept() {
      if (consumed) return false;
      consumed = true;
      if (epoch !== generation || pending || getOperator()?.id !== captured.operator_id) return false;
      // The target or authored choice may have disappeared during confirmation.
      // Submit the captured intent so canonical admission records its refusal.
      const request = { ...captured, correlation: correlation() };
      let accepted = false;
      try { accepted = submit(request) !== false; } catch (_) { /* visible refusal */ }
      if (!accepted) { feedback('refused'); return false; }
      pending = request; feedback('pending'); options(); render();
      timer = schedule(() => { timer = null; pending = null; feedback('timed_out'); options(); render(); }, DEFAULT_ACTION_FEEDBACK_TIMEOUT_MS);
      return true;
    } }) !== false;
  }
  function update(payload) {
    const parsed = parseNpcDoctrinePayload(payload); if (!parsed) return false;
    ({ entities, profiles, results } = parsed);
    if (selected) selected = entities.find(row => row.entity_id === selected.entity_id) || null;
    const terminal = pending && results.find(row => row.operator_id === pending.operator_id && row.correlation === pending.correlation
      && row.target === pending.target && row.npc_doctrine === pending.doctrine);
    if (terminal) {
      if (timer !== null) cancelSchedule(timer);
      timer = null; pending = null; feedback(terminal.outcome === 'no-op' ? 'no_op' : terminal.outcome);
    }
    if (el('results')) {
      el('results').replaceChildren();
      for (const result of results) {
        const row = doc.createElement('li'); row.dataset.outcome = result.outcome; row.dataset.target = result.target;
        row.dataset.correlation = result.correlation; row.dataset.doctrine = result.npc_doctrine;
        row.textContent = t('server.gm.npc.result', { operator: result.operator_id, target: result.target, doctrine: result.npc_doctrine,
          tick: result.tick, outcome: t(`server.gm.npc.${result.outcome === 'no-op' ? 'no_op' : result.outcome}`),
          reason: result.reason ? t(GM_ACTION_REFUSAL_REASON_LABELS[result.reason] || 'server.gm.effect.reason_unknown', { reason: result.reason }) : '' });
        el('results').appendChild(row);
      }
    }
    options(); render(); return true;
  }
  function reset() {
    generation++; if (timer !== null) cancelSchedule(timer);
    timer = null; pending = null; selected = null; choice = ''; profiles = {}; entities = []; results = [];
    if (el('feedback')) el('feedback').textContent = '';
    el('results')?.replaceChildren(); options(); render();
  }
  el('choice')?.addEventListener('change', () => { choice = el('choice').value; render(); });
  el('apply')?.addEventListener('click', choose);
  render();
  return { choose, update, reset, refreshAdmission: render,
    select(entity) { if (selected?.entity_id !== entity?.entity_id) { generation++; choice = ''; } selected = entity; options(); render(); },
    state: () => ({ selected, choice, profiles, pending, results }) };
}
