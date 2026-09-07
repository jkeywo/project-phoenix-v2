/** Authored Objective controls. The absolute projection owns every status and recipient. */
import { has } from './strings.js';
import { GM_ACTION_REFUSAL_REASON_LABELS } from './gm-action-reasons.js';
import {
  ActionFeedbackLifecycle, ACTION_FEEDBACK_STATE, DEFAULT_ACTION_FEEDBACK_TIMEOUT_MS,
} from './action-feedback.js';

const VERBS = ['activate', 'complete', 'fail'];
const STATUSES = ['Active', 'Completed', 'Failed'];
const OUTCOMES = ['applied', 'no-op', 'refused'];
const nonempty = (value) => typeof value === 'string' && value.length > 0;
const scopeValid = (value) => Array.isArray(value) && value.every(nonempty)
  && new Set(value).size === value.length;
const sameScope = (a, b) => a.length === b.length && a.every((id) => b.includes(id));

function parseObjective(row, palette) {
  if (!row || !nonempty(row.id) || !nonempty(row.text)
      || !scopeValid(row.recipients) || typeof row.available !== 'boolean'
      || !(STATUSES.includes(row.status) || (palette && row.status === null))
      || (palette && !nonempty(row.label))
      || (row.text_params != null && (typeof row.text_params !== 'object'
        || Array.isArray(row.text_params) || !Object.values(row.text_params).every((v) => typeof v === 'string')))) return null;
  return { id: row.id, text: row.text, text_params: { ...(row.text_params || {}) },
    recipients: [...row.recipients], available: row.available, status: row.status,
    ...(palette ? { label: row.label } : {}) };
}

export function parseGmObjectivePayload(payload) {
  let value = payload;
  if (typeof value === 'string') { try { value = JSON.parse(value); } catch (_) { return null; } }
  if (!value || !Array.isArray(value.objective_palette) || !Array.isArray(value.objectives)
      || !Array.isArray(value.objective_results)) return null;
  const palette = value.objective_palette.map((row) => parseObjective(row, true));
  const objectives = value.objectives.map((row) => parseObjective(row, false));
  if ([...palette, ...objectives].some((row) => !row)
      || new Set(palette.map((row) => row.id)).size !== palette.length
      || new Set(objectives.map((row) => row.id)).size !== objectives.length) return null;
  const results = [];
  for (const row of value.objective_results) {
    if (!row || row.action_kind !== 'objective-control' || !nonempty(row.target)
        || !nonempty(row.operator_id) || !nonempty(row.correlation)
        || !VERBS.includes(row.objective_verb) || !scopeValid(row.objective_recipients)
        || !OUTCOMES.includes(row.outcome) || !Number.isSafeInteger(row.tick) || row.tick < 0
        || (row.reason != null && !nonempty(row.reason))) return null;
    results.push({ ...row, objective_recipients: [...row.objective_recipients] });
  }
  return { palette, objectives, results };
}

export function createGmObjectivePanel({ doc = globalThis.document, t = (id) => id,
  getOperator = () => null, getOperatorName = (id) => id, getShipName = (id) => id,
  submit = () => false, correlation, actionFeedback: suppliedFeedback, confirmAction = null,
  schedule = globalThis.setTimeout, cancelSchedule = globalThis.clearTimeout } = {}) {
  const el = (suffix) => doc?.getElementById(`gm-objective-${suffix}`);
  const actionFeedback = suppliedFeedback || new ActionFeedbackLifecycle({ ...(correlation ? { correlation } : {}) });
  let projection = { palette: [], objectives: [], results: [] };
  let preview = null;
  let pending = null;
  let timer = null;
  let opener = null;
  const text = (value, params) => has(value) ? t(value, params) : value;
  const scopeText = (recipients) => recipients.length ? recipients.map(getShipName).join(', ')
    : t('server.gm.objective.all_ships');
  const rowFor = (id, verb) => (verb === 'activate' ? projection.palette : projection.objectives)
    .find((row) => row.id === id);
  const eligible = (row, verb) => !!getOperator()?.id && !pending && row?.available === true
    && (verb === 'activate' ? row.status === null : row.status === 'Active');
  function closePreview(restoreFocus = false) {
    preview = null;
    if (el('confirmation')) el('confirmation').hidden = true;
    if (restoreFocus && opener) [...(el('list')?.querySelectorAll('button[data-verb]') || [])]
      .find((button) => button.dataset.objective === opener.id && button.dataset.verb === opener.verb)?.focus();
  }
  function feedback(state, id = '') {
    if (!el('feedback')) return;
    el('feedback').dataset.state = state;
    el('feedback').dataset.objective = id;
    el('feedback').textContent = state ? t(`server.gm.objective.feedback_${state}`) : '';
  }
  function refreshAdmission() {
    for (const button of el('list')?.querySelectorAll('button[data-verb]') || []) {
      button.disabled = !eligible(rowFor(button.dataset.objective, button.dataset.verb), button.dataset.verb);
    }
    if (preview) {
      const row = rowFor(preview.id, preview.verb);
      if (!eligible(row, preview.verb) || !sameScope(row.recipients, preview.recipients)
          || row.text !== preview.text || JSON.stringify(row.text_params) !== JSON.stringify(preview.text_params)
          || getOperator()?.id !== preview.operator) closePreview();
    }
  }
  function openPreview(id, verb) {
    const row = rowFor(id, verb);
    if (!VERBS.includes(verb) || !eligible(row, verb)) return false;
    const chosen = { ...row, recipients: [...row.recipients], verb, operator: getOperator().id };
    const description = t(`server.gm.objective.preview_${verb}`, {
      objective: text(row.text, row.text_params), ships: scopeText(row.recipients),
    });
    if (confirmAction) return confirmAction({ category: `objective.${verb}`, description,
      preview: () => description, accept: () => submitChosen(chosen) });
    preview = chosen;
    opener = { id, verb };
    if (el('consequence')) el('consequence').textContent = description;
    if (el('confirmation')) el('confirmation').hidden = false;
    el('confirm')?.focus();
    return true;
  }
  function confirm() {
    refreshAdmission();
    if (!preview) return false;
    const chosen = preview;
    closePreview();
    return submitChosen(chosen);
  }
  function submitChosen(chosen) {
    if (pending || getOperator()?.id !== chosen.operator) return false;
    const action = actionFeedback.press(`gm.objective.${chosen.verb}:${chosen.id}`);
    const request = { operator_id: chosen.operator, correlation: action.correlation,
      objective: chosen.id, verb: chosen.verb, recipients: [...chosen.recipients] };
    pending = request;
    actionFeedback.pending(request.correlation);
    let accepted = false;
    try { accepted = submit(request) !== false; } catch (_) { /* visible local refusal */ }
    if (!accepted) {
      actionFeedback.settle(request.correlation, ACTION_FEEDBACK_STATE.REFUSED);
      pending = null; feedback('refused', chosen.id); refreshAdmission(); return false;
    }
    feedback('pending', chosen.id);
    timer = schedule(() => {
      actionFeedback.settle(request.correlation, ACTION_FEEDBACK_STATE.TIMED_OUT);
      pending = null; timer = null; feedback('timed_out', chosen.id); refreshAdmission();
    }, DEFAULT_ACTION_FEEDBACK_TIMEOUT_MS);
    refreshAdmission(); return true;
  }
  function renderRows() {
    const list = el('list');
    if (!list) return;
    // Preserve focused controls across periodic absolute projections.
    const focused = list.contains(doc.activeElement) ? { ...doc.activeElement.dataset } : null;
    list.replaceChildren();
    const live = new Map(projection.objectives.map((row) => [row.id, row]));
    const rows = projection.palette.map((row) => ({ ...row, ...(live.get(row.id) || {}), palette: true }));
    for (const row of projection.objectives) if (!projection.palette.some((p) => p.id === row.id)) rows.push(row);
    for (const objective of rows) {
      const row = doc.createElement('li'); row.dataset.objective = objective.id;
      row.dataset.status = objective.status || 'Unstarted'; row.className = 'gm-objective-row';
      if (objective.label && objective.label !== objective.text) {
        const label = doc.createElement('h4'); label.textContent = text(objective.label); row.appendChild(label);
      }
      const description = doc.createElement('p');
      description.textContent = text(objective.text, objective.text_params); row.appendChild(description);
      const scope = doc.createElement('p');
      scope.textContent = t('server.gm.objective.scope', { ships: scopeText(objective.recipients),
        status: t(`server.gm.objective.status_${objective.available ? objective.status || 'Unstarted' : 'Unavailable'}`) });
      row.appendChild(scope);
      for (const verb of VERBS) {
        if (verb === 'activate' && !objective.palette) continue;
        const button = doc.createElement('button'); button.type = 'button';
        button.dataset.objective = objective.id; button.dataset.verb = verb;
        button.dataset.actionId = `gm.objective.${verb}:${objective.id}`;
        button.textContent = t(`server.gm.objective.${verb}`);
        button.setAttribute('aria-label', t('server.gm.objective.action_label', {
          verb: button.textContent, objective: text(objective.label || objective.text, objective.text_params),
        }));
        button.addEventListener('click', () => openPreview(objective.id, verb));
        row.appendChild(button);
      }
      list.appendChild(row);
    }
    if (el('empty')) el('empty').hidden = rows.length > 0;
    refreshAdmission();
    if (focused) [...list.querySelectorAll('button')].find((b) => b.dataset.objective === focused.objective
      && b.dataset.verb === focused.verb)?.focus();
  }
  function renderResults() {
    const list = el('results');
    if (!list) return;
    list.replaceChildren();
    for (const result of projection.results) {
      const row = doc.createElement('li');
      Object.assign(row.dataset, { objective: result.target, verb: result.objective_verb,
        outcome: result.outcome, operatorId: result.operator_id, correlation: result.correlation, tick: String(result.tick) });
      if (result.reason) row.dataset.reason = result.reason;
      row.textContent = t('server.gm.objective.result', { operator: getOperatorName(result.operator_id),
        verb: t(`server.gm.objective.${result.objective_verb}`), objective: result.target,
        ships: scopeText(result.objective_recipients), tick: result.tick, correlation: result.correlation,
        outcome: t(`server.gm.objective.feedback_${result.outcome.replace('-', '_')}`),
        reason: result.reason ? t(GM_ACTION_REFUSAL_REASON_LABELS[result.reason]
          || 'server.gm.effect.reason_unknown', { reason: result.reason }) : '' });
      list.appendChild(row);
    }
  }
  function update(payload) {
    const next = parseGmObjectivePayload(payload);
    if (!next) return false;
    projection = next;
    const result = pending && projection.results.find((r) => r.operator_id === pending.operator_id
      && r.correlation === pending.correlation && r.target === pending.objective
      && r.objective_verb === pending.verb && sameScope(r.objective_recipients, pending.recipients));
    if (result) {
      if (timer !== null) cancelSchedule(timer);
      timer = null;
      actionFeedback.settle(pending.correlation, result.outcome === 'refused'
        ? ACTION_FEEDBACK_STATE.REFUSED : ACTION_FEEDBACK_STATE.APPLIED);
      pending = null; feedback(result.outcome.replace('-', '_'), result.target);
    }
    renderRows(); renderResults(); return true;
  }
  function reset() {
    if (timer !== null) cancelSchedule(timer);
    if (pending) actionFeedback.cancel(pending.correlation);
    timer = null; pending = null; projection = { palette: [], objectives: [], results: [] };
    closePreview(); feedback(''); renderRows(); renderResults();
  }
  el('confirm')?.addEventListener('click', confirm);
  el('cancel')?.addEventListener('click', () => closePreview(true));
  el('confirmation')?.addEventListener('keydown', (event) => {
    if (event.key === 'Escape') { event.preventDefault(); closePreview(true); }
  });
  renderRows();
  return { update, confirm, reset, refreshAdmission, state: () => ({ ...projection, preview, pending }) };
}
