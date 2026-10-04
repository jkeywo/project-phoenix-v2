import { GmActionFeedback } from './gm-action-feedback.js';
/** Ordinary Comms Studio, with immutable intent before private confirmation. */
import { ActionFeedbackLifecycle, createActionCorrelation, DEFAULT_ACTION_FEEDBACK_TIMEOUT_MS } from './action-feedback.js';
import { GM_ACTION_REFUSAL_REASON_LABELS } from './gm-action-reasons.js';

export function createGmCommsPanel({ doc = globalThis.document, t = id => id,
  getOperator = () => null, getOperatorName = id => id, submitTransmission = () => false,
  confirmAction = request => request.accept(), correlation = createActionCorrelation,
  actionFeedback = new ActionFeedbackLifecycle({ correlation }),
  schedule = setTimeout, cancelSchedule = clearTimeout,
  timeoutMs = DEFAULT_ACTION_FEEDBACK_TIMEOUT_MS,
} = {}) {
  const byId = id => doc.getElementById(`gm-comms-${id}`);
  const routeInput = byId('route'), senderInput = byId('sender'), recipientsInput = byId('recipients');
  const textInput = byId('text'), hailInput = byId('hail'), sendButton = byId('send'), hailButton = byId('start-hail');
  const feedback = byId('feedback'), log = byId('log');
  let projection = { routes: [], recipients: [], results: [], max_text_bytes: 0 };
  let generation = 0;
  const pending = new GmActionFeedback({ lifecycle: () => actionFeedback, capacity: 32,
    timeoutMs, schedule, cancelSchedule, onLocalTerminal: (_meta, outcome) => {
      paintFeedback(outcome === 'timed-out' ? 'TimedOut' : 'Refused'); renderLog(); refreshAdmission();
    } });
  const key = row => `${row.operator_id}\n${row.correlation}`;
  const route = () => projection.routes.find(row => row.id === routeInput?.value);
  const wireText = value => t(value) || value;
  const identityName = row => row ? `${wireText(row.name || row.label)}${row.fleet_slot == null ? '' : ` (${row.fleet_slot})`}` : '';
  const bytes = value => new TextEncoder().encode(value).length;
  function options(input, rows, selected = [...(input?.selectedOptions || [])].map(o => o.value)) {
    if (!input) return;
    const signature = JSON.stringify(rows);
    if (input.dataset.options === signature) return;
    input.dataset.options = signature;
    // Keep an invalidated draft choice visibly selected. Native single-select
    // fallback to the first survivor would silently change its fictional
    // sender, route or hail while leaving the operator's text intact.
    const stale = [...input.options].filter(option => selected.includes(option.value)
      && !rows.some(row => row.id === option.value)).map(option => {
      const copy = doc.createElement('option'); copy.value = option.value;
      copy.textContent = option.textContent; copy.disabled = true; copy.selected = true;
      return copy;
    });
    input.replaceChildren(...rows.map(row => {
      const option = doc.createElement('option'); option.value = row.id;
      option.textContent = identityName(row); option.selected = selected.includes(row.id);
      return option;
    }), ...stale);
    if (!input.multiple && selected.length === 1) input.value = selected[0];
  }
  function selectedRecipients() {
    return route()?.visibility === 'fleet' ? projection.recipients.map(row => row.id).sort()
      : [...(recipientsInput?.selectedOptions || [])].map(row => row.value).sort();
  }
  function valid(intent) {
    const choice = projection.routes.find(row => row.id === intent.route);
    return choice && choice.senders.some(row => row.id === intent.sender)
      && intent.recipients.length > 0 && intent.recipients.length <= 32
      && intent.recipients.every(id => projection.recipients.some(row => row.id === id))
      && (choice.visibility !== 'fleet' || JSON.stringify(intent.recipients) === JSON.stringify(projection.recipients.map(row => row.id).sort()))
      && (intent.content.literal ? intent.content.literal.text.length > 0 && bytes(intent.content.literal.text) <= projection.max_text_bytes
        : choice.hails.some(row => row.id === intent.content.scripted_hail?.hail));
  }
  function intentFor(hail) {
    return { route: routeInput?.value, sender: senderInput?.value, recipients: selectedRecipients(),
      content: hail ? { scripted_hail: { hail: hailInput?.value } } : { literal: { text: textInput?.value || '' } } };
  }
  function refreshAdmission() {
    const choice = route();
    options(senderInput, choice?.senders || []);
    options(hailInput, choice?.hails || []);
    if (recipientsInput) {
      recipientsInput.disabled = choice?.visibility === 'fleet';
      if (choice?.visibility === 'fleet') for (const option of recipientsInput.options) option.selected = true;
    }
    if (byId('count')) byId('count').textContent = t('server.gm.comms.byte_count', { count: bytes(textInput?.value || ''), max: projection.max_text_bytes });
    if (byId('empty')) byId('empty').hidden = projection.routes.length > 0;
    const admitted = !!getOperator()?.id && pending.size < 32;
    if (sendButton) sendButton.disabled = !admitted || !valid(intentFor(false));
    if (hailButton) hailButton.disabled = !admitted || !valid(intentFor(true));
  }
  function paintFeedback(state) {
    if (!feedback) return;
    feedback.dataset.state = state;
    feedback.textContent = t(`action_feedback.${{ Pending: 'pending', Applied: 'applied', Refused: 'refused', TimedOut: 'timed_out' }[state] || 'refused'}`);
  }
  function renderLog() {
    if (!log) return;
    const rows = new Map(projection.results.map(row => [key(row), row]));
    for (const row of [...pending.localRequests(), ...pending.requests()]) if (!rows.has(key(row))) rows.set(key(row), row);
    log.replaceChildren(...[...rows.values()].slice(-128).map(row => {
      const item = doc.createElement('li'); item.dataset.correlation = row.correlation;
      item.dataset.operatorId = row.operator_id; item.dataset.outcome = row.outcome;
      const intent = row.transmission;
      const reason = row.reason ? t(GM_ACTION_REFUSAL_REASON_LABELS[row.reason] || 'server.gm.comms.refused') : '';
      const names = (intent?.recipients || []).map(id => identityName(projection.recipients.find(ship => ship.id === id)) || id).join(', ');
      const choice = projection.routes.find(route => route.id === intent?.route);
      const senderName = identityName(choice?.senders.find(sender => sender.id === intent?.sender)) || intent?.sender || '';
      const outcome = row.outcome === 'applied' && intent?.content.scripted_hail ? 'hail_queued' : row.outcome;
      item.textContent = t('server.gm.comms.result', { operator: getOperatorName(row.operator_id),
        outcome: t(`server.gm.comms.outcome.${outcome}`), reason, sender: senderName, recipients: names, correlation: row.correlation });
      if (intent) {
        const body = doc.createElement('pre');
        // Exact runtime prose never passes through t(), HTML or interpolation.
        body.textContent = intent.content.literal?.text ?? wireText(choice?.hails.find(h => h.id === intent.content.scripted_hail?.hail)?.name || intent.content.scripted_hail?.hail || '');
        item.appendChild(body);
      }
      return item;
    }));
  }
  function requestSend(hail = false) {
    const operator = getOperator(); const intent = intentFor(hail); const born = generation;
    if (!operator?.id || !valid(intent)) return false;
    const operatorId = operator.id;
    const senderName = wireText(route()?.senders.find(row => row.id === intent.sender)?.name || intent.sender);
    const recipientNames = intent.recipients.map(id => identityName(projection.recipients.find(row => row.id === id)) || id).join(', ');
    const previewText = intent.content.literal?.text ?? wireText(route()?.hails.find(row => row.id === intent.content.scripted_hail?.hail)?.name || '');
    let submitted = false;
    return confirmAction({ category: 'comms.send', description: t(hail ? 'server.gm.comms.start_hail' : 'server.gm.comms.send'),
      preview: () => t('server.gm.comms.preview', {
        sender: senderName, recipients: recipientNames, text: previewText,
      }),
      accept: () => {
        if (submitted || generation !== born || getOperator()?.id !== operatorId || pending.size >= 32) return false;
        // A lost sender, route or recipient is still this captured intent.
        // Ordinary admission must record its attributed refusal on acceptance.
        submitted = true;
        const press = actionFeedback.press('gm.comms.send');
        const meta = { operator_id: operatorId, correlation: press.correlation, transmission: intent, outcome: 'pending' };
        const tracked = pending.trackRequest(meta);
        if (!tracked) return false;
        paintFeedback('Pending');
        let accepted = false;
        try { accepted = submitTransmission({ operator_id: operatorId, correlation: press.correlation, transmission: intent }) === true; } catch { /* ingress refuses */ }
        pending.submitted(tracked, accepted, 'ingress-rejected');
        renderLog(); refreshAdmission(); return accepted;
      },
    });
  }
  function update(payload) {
    if (typeof payload === 'string') {
      try { payload = JSON.parse(payload); } catch { return false; }
    }
    if (!payload || !Array.isArray(payload.routes) || !Array.isArray(payload.recipients) || !Array.isArray(payload.results)
      || !Number.isSafeInteger(payload.max_text_bytes) || payload.max_text_bytes <= 0) return false;
    projection = structuredClone(payload);
    options(routeInput, projection.routes); options(recipientsInput, projection.recipients);
    for (const result of projection.results) {
      const meta = pending.get(result.correlation);
      if (meta && meta.operatorId === result.operator_id && ['applied','refused','no-op'].includes(result.outcome)) {
        const settled = pending.settle(result);
        pending.remember(settled.meta, result.outcome, result.reason);
        paintFeedback(settled.state);
      }
    }
    renderLog(); refreshAdmission(); return true;
  }
  function reset() {
    generation++;
    pending.reset(); projection = { routes: [], recipients: [], results: [], max_text_bytes: 0 };
    for (const input of [routeInput, senderInput, recipientsInput, hailInput]) {
      if (input) { input.replaceChildren(); delete input.dataset.options; }
    }
    if (textInput) textInput.value = '';
    if (feedback) { feedback.textContent = ''; delete feedback.dataset.state; }
    renderLog(); refreshAdmission();
  }
  for (const input of [routeInput, senderInput, recipientsInput, hailInput]) input?.addEventListener('change', () => {
    // A deliberate new multi-selection drops unavailable recipients from the
    // draft; a background projection alone cannot narrow its audience.
    if (input.multiple) for (const option of input.options) if (option.disabled) option.selected = false;
    refreshAdmission();
  });
  textInput?.addEventListener('input', refreshAdmission);
  sendButton?.addEventListener('click', () => requestSend());
  hailButton?.addEventListener('click', () => requestSend(true));
  refreshAdmission();
  /**
   * Bring one authored route to the operator's hand (issue #1433).
   *
   * The GM attention queue opens a pending conversation by naming the route
   * that already speaks as its sender — this is that navigation and nothing
   * more: it selects an existing option, repaints the dependent selects, and
   * puts focus where a reply would be composed. It submits nothing, and a
   * route this projection does not hold is simply refused.
   */
  function focusRoute(routeId) {
    if (!routeInput || !projection.routes.some(row => row.id === routeId)) return false;
    routeInput.value = routeId;
    refreshAdmission();
    doc.getElementById('gm-comms-panel')?.scrollIntoView?.({ block: 'nearest' });
    routeInput.focus({ preventScroll: true });
    return true;
  }
  return { update, reset, refreshAdmission, requestSend, focusRoute,
    refreshLanguage() { renderLog(); refreshAdmission(); },
    state: () => structuredClone({ ...projection, pending: [...pending.requests()] }) };
}
