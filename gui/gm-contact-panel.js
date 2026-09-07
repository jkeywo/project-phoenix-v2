import { createActionCorrelation, DEFAULT_ACTION_FEEDBACK_TIMEOUT_MS } from './action-feedback.js';
import { wireText } from './strings.js';

const MODES = ['reveal', 'conceal', 'normal'];

/** One observing player ship and one real world target; absolute state survives reconnect. */
export function createGmContactPanel({ doc = globalThis.document, t = id => id,
  getOperator = () => null, submit = () => false, correlation = createActionCorrelation,
  confirmAction = request => request.accept(),
  schedule = globalThis.setTimeout, cancelSchedule = globalThis.clearTimeout } = {}) {
  const el = suffix => doc?.getElementById(`gm-contact-${suffix}`);
  let selected = null, observer = '', entities = [], overrides = {}, pending = null, timer = null;
  const valid = () => getOperator() && !pending && selected && selected.entity_id !== observer
    && entities.some(row => row.entity_id === observer && row.kind === 'player_ship')
    && entities.some(row => row.entity_id === selected.entity_id);
  function render() {
    if (el('target')) el('target').textContent = selected ? wireText(selected.name) : t('server.gm.contact.select');
    if (el('mode')) el('mode').textContent = t(`server.gm.contact.${overrides[observer]?.[selected?.entity_id] || 'normal'}`);
    for (const mode of MODES) if (el(mode)) el(mode).disabled = !valid();
  }
  function feedback(state) {
    if (el('feedback')) { el('feedback').dataset.state = state; el('feedback').textContent = t(`server.gm.contact.${state}`); }
  }
  function choose(mode) {
    if (!MODES.includes(mode) || !valid()) return false;
    const chosen = { operator_id: getOperator().id, ship: observer,
      target: selected.entity_id, mode };
    const description = t('settings.gm.confirmation.contact', {
      mode: t(`server.gm.contact.${mode}`), target: wireText(selected.name),
      ship: wireText(entities.find(row => row.entity_id === observer)?.name || observer),
    });
    return confirmAction({ category: 'contact.override', description, preview: () => description,
      accept: () => submitChosen(chosen) });
  }
  function submitChosen(chosen) {
    if (pending || getOperator()?.id !== chosen.operator_id) return false;
    const request = { ...chosen, correlation: correlation() };
    let accepted = false;
    try { accepted = submit(request) !== false; } catch (_) { /* report below */ }
    if (!accepted) { feedback('refused'); return false; }
    pending = request;
    feedback('pending');
    timer = schedule(() => { pending = null; timer = null; feedback('timed_out'); render(); }, DEFAULT_ACTION_FEEDBACK_TIMEOUT_MS);
    render(); return true;
  }
  function update(payload) {
    let value = payload;
    if (typeof value === 'string') { try { value = JSON.parse(value); } catch (_) { return false; } }
    if (!value || !Array.isArray(value.entities) || !Array.isArray(value.contact_results || [])) return false;
    const rows = value.contact_results || [];
    if (rows.some(row => !row || !MODES.some(mode => row.action_kind === `contact-${mode}`)
      || typeof row.observer !== 'string' || !row.observer || typeof row.target !== 'string' || !row.target
      || typeof row.operator_id !== 'string' || !row.operator_id || typeof row.correlation !== 'string' || !row.correlation
      || !Number.isSafeInteger(row.tick) || !['applied', 'no-op', 'refused'].includes(row.outcome))) return false;
    const nextOverrides = value.contact_overrides || {};
    if (typeof nextOverrides !== 'object' || Object.values(nextOverrides).some(targets => !targets || typeof targets !== 'object'
      || Object.values(targets).some(mode => !MODES.includes(mode)))) return false;
    entities = value.entities; overrides = nextOverrides;
    const select = el('observer');
    if (select) {
      select.replaceChildren();
      const placeholder = doc.createElement('option'); placeholder.value = ''; placeholder.textContent = t('server.gm.contact.observer'); select.appendChild(placeholder);
      for (const ship of entities.filter(row => row.kind === 'player_ship')) {
        const option = doc.createElement('option'); option.value = ship.entity_id; option.textContent = wireText(ship.name); select.appendChild(option);
      }
      if (!entities.some(row => row.kind === 'player_ship' && row.entity_id === observer)) observer = '';
      select.value = observer;
    }
    if (selected) selected = entities.find(row => row.entity_id === selected.entity_id) || null;
    const terminal = pending && rows.find(row => row.operator_id === pending.operator_id && row.correlation === pending.correlation
      && row.observer === pending.ship && row.target === pending.target && row.action_kind === `contact-${pending.mode}`);
    if (terminal) {
      if (timer !== null) cancelSchedule(timer);
      timer = null; pending = null; feedback(terminal.outcome === 'refused' ? 'refused' : 'applied');
    }
    const list = el('results');
    if (list) {
      list.replaceChildren();
      for (const row of rows) {
        const item = doc.createElement('li'); item.dataset.outcome = row.outcome;
        item.textContent = t('server.gm.contact.result', { operator: row.operator_id, observer: row.observer, target: row.target,
          mode: t(`server.gm.contact.${row.action_kind.slice(8)}`), tick: row.tick, outcome: t(`server.gm.contact.outcome_${row.outcome.replace('-', '_')}`) });
        list.appendChild(item);
      }
    }
    render(); return true;
  }
  function reset() {
    if (timer !== null) cancelSchedule(timer);
    timer = null; pending = null; selected = null; observer = ''; entities = []; overrides = {};
    el('results')?.replaceChildren(); if (el('feedback')) el('feedback').textContent = ''; render();
  }
  el('observer')?.addEventListener('change', () => { observer = el('observer').value; render(); });
  for (const mode of MODES) el(mode)?.addEventListener('click', () => choose(mode));
  render();
  return { update, choose, reset, refreshAdmission: render, select: entity => { selected = entity; render(); },
    state: () => ({ observer, target: selected?.entity_id || null, pending, overrides }) };
}
