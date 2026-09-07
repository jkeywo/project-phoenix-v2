/** Selected-entity removal preview. Only canonical results remove world state. */
import { GM_ACTION_REFUSAL_REASON_LABELS } from './gm-action-reasons.js';
import { has } from './strings.js';
import { createActionCorrelation, DEFAULT_ACTION_FEEDBACK_TIMEOUT_MS } from './action-feedback.js';

export function createGmDespawnPanel({ doc = globalThis.document, t = (id) => id,
  getOperator = () => null, submit = () => false, correlation = createActionCorrelation,
  confirmAction = null,
  schedule = globalThis.setTimeout, cancelSchedule = globalThis.clearTimeout } = {}) {
  const el = (suffix) => doc?.getElementById(`gm-despawn-${suffix}`);
  let selected = null;
  let previewId = null;
  let pending = null;
  let timer = null;
  let results = [];
  const name = (entity) => has(entity.name) ? t(entity.name) : entity.name;
  const eligible = () => selected?.removable === true && !!getOperator() && !pending;
  function feedback(state) {
    if (!el('feedback')) return;
    el('feedback').dataset.state = state;
    el('feedback').textContent = t(`server.gm.despawn.${state}`);
  }
  function closePreview() {
    previewId = null;
    if (el('confirmation')) el('confirmation').hidden = true;
  }
  function render() {
    if (el('target')) el('target').textContent = selected
      ? t(selected.removable ? 'server.gm.despawn.target' : 'server.gm.despawn.protected', { name: name(selected) })
      : t('server.gm.despawn.select');
    if (el('preview')) el('preview').disabled = !eligible();
    if (!eligible() || selected?.entity_id !== previewId) closePreview();
  }
  function preview() {
    if (!eligible()) return false;
    previewId = selected.entity_id;
    if (confirmAction) {
      const description = t('server.gm.despawn.consequence', { name: name(selected) });
      return confirmAction({ category: 'world.despawn', description,
        preview: () => description, accept: confirm,
      });
    }
    if (el('consequence')) el('consequence').textContent = t('server.gm.despawn.consequence', { name: name(selected) });
    if (el('confirmation')) el('confirmation').hidden = false;
    el('confirm')?.focus();
    return true;
  }
  function confirm() {
    if (!eligible() || previewId !== selected.entity_id) { closePreview(); return false; }
    const operator = getOperator();
    const request = { operator_id: operator.id, correlation: correlation(), target: previewId };
    closePreview();
    let accepted = false;
    try { accepted = submit(request) !== false; } catch (_) { /* visible refusal below */ }
    if (!accepted) { feedback('refused'); render(); return false; }
    pending = request;
    feedback('pending');
    timer = schedule(() => { pending = null; timer = null; feedback('timed_out'); render(); }, DEFAULT_ACTION_FEEDBACK_TIMEOUT_MS);
    render();
    return true;
  }
  function update(payload) {
    let value = payload;
    if (typeof value === 'string') { try { value = JSON.parse(value); } catch (_) { return false; } }
    if (!value || (value.despawn_results != null && !Array.isArray(value.despawn_results))) return false;
    const next = value.despawn_results || [];
    if (next.some((r) => !r || r.action_kind !== 'world-despawn' || typeof r.target !== 'string' || !r.target
        || typeof r.operator_id !== 'string' || !r.operator_id || typeof r.correlation !== 'string' || !r.correlation
        || !Number.isSafeInteger(r.tick) || r.tick < 0 || (r.reason != null && typeof r.reason !== 'string')
        || !['applied', 'no-op', 'refused'].includes(r.outcome))) return false;
    results = next;
    const terminal = pending && results.find((r) => r.operator_id === pending.operator_id && r.correlation === pending.correlation);
    if (terminal) {
      if (timer !== null) cancelSchedule(timer);
      timer = null; pending = null;
      feedback(terminal.outcome === 'refused' ? 'refused' : 'applied');
    }
    const list = el('results');
    if (list) {
      list.replaceChildren();
      for (const result of results) {
        const row = doc.createElement('li');
        row.dataset.outcome = result.outcome;
        row.dataset.target = result.target;
        row.textContent = t('server.gm.despawn.result', { operator: result.operator_id, target: result.target,
          tick: result.tick, outcome: t(`server.gm.despawn.${result.outcome === 'no-op' ? 'no_op' : result.outcome}`),
          reason: result.reason ? t(GM_ACTION_REFUSAL_REASON_LABELS[result.reason] || 'server.gm.effect.reason_unknown', { reason: result.reason }) : '' });
        list.appendChild(row);
      }
    }
    render(); return true;
  }
  function reset() {
    if (timer !== null) cancelSchedule(timer);
    timer = null; pending = null; selected = null; results = [];
    closePreview(); el('results')?.replaceChildren();
    if (el('feedback')) el('feedback').textContent = '';
    render();
  }
  el('preview')?.addEventListener('click', preview);
  el('confirm')?.addEventListener('click', confirm);
  el('cancel')?.addEventListener('click', () => { closePreview(); el('preview')?.focus(); });
  el('confirmation')?.addEventListener('keydown', (event) => {
    if (event.key === 'Escape') { closePreview(); el('preview')?.focus(); }
  });
  render();
  return { select: (entity) => { selected = entity; render(); }, preview, confirm, update, reset,
    refreshAdmission: render, state: () => ({ selected, previewId, pending, results }) };
}
