import { createActionCorrelation, DEFAULT_ACTION_FEEDBACK_TIMEOUT_MS } from './action-feedback.js';
import { wireText } from './strings.js';

const MODES = ['reveal', 'conceal', 'normal'];
const RESULT_MODES = [...MODES, 'misclassify', 'classification-normal', 'information'];
const informationTarget = change => { const row = Object.values(change || {})[0]; return row?.id || row?.target; };
const requestedKind = request => request.change ? 'contact-information' : Object.hasOwn(request, 'palette') ? (request.palette === null ? 'contact-classification-normal' : 'contact-misclassify') : `contact-${request.mode}`;

/** One observing player ship and one real world target; absolute state survives reconnect. */
export function createGmContactPanel({ doc = globalThis.document, t = id => id,
  getOperator = () => null, submit = () => false, submitClassification = () => false, submitInformation = () => false, correlation = createActionCorrelation,
  confirmAction = request => request.accept(),
  // Each of the three drafts this panel carries finishes on its own applied
  // outcome (issue #1510). Mode changes, clears and removals are verbs.
  onSucceeded = () => {},
  // Picking a ghost's place covers the chart with a gesture, so the surface
  // puts its other floating panels away for the duration (issue #1508).
  // Bring a draft panel on screen. The surface owns whether a panel is open;
  // this panel only ever asks.
  schedule = globalThis.setTimeout, cancelSchedule = globalThis.clearTimeout } = {}) {
  const el = suffix => doc?.getElementById(`gm-contact-${suffix}`);
  const keepOpen = draft => el(`${draft}-keep-open`)?.checked === true;
  let selected = null, observer = '', entities = [], overrides = {}, classifications = {}, palette = [], information = { ghosts: {} }, pending = null, timer = null;
  let renderedPalette = null, renderedObservers = null, renderedGhosts = null;
  /** The picker speaks metres; a ghost's position is canonical integer
   * millimetres. The bounds themselves do not move: the converted value lands
   * in the same fields, checked by the same `validPosition`. */
  const validObserver = () => getOperator() && !pending && entities.some(row => row.entity_id === observer && row.kind === 'player_ship');
  const boundedId = value => typeof value === 'string' && value.length > 0 && new TextEncoder().encode(value).length <= 128 && !/[\u0000-\u001f\u007f-\u009f]/u.test(value);
  const validPosition = values => values.every(value => Number.isInteger(value) && value >= -2147483648 && value <= 2147483647);
  const valid = () => getOperator() && !pending && selected && selected.entity_id !== observer
    && entities.some(row => row.entity_id === observer && row.kind === 'player_ship')
    && entities.some(row => row.entity_id === selected.entity_id);
  const validPolicy = policy => policy && [policy.delay_ticks, policy.position_step_mm].every(value => Number.isInteger(value) && value >= 0 && value <= 4294967295) && typeof policy.hide_identity === 'boolean' && (policy.delay_ticks > 0 || policy.position_step_mm > 0 || policy.hide_identity);
  const chosenPolicy = () => ({ delay_ticks: Number(el('report-delay')?.value), position_step_mm: Number(el('report-step')?.value), hide_identity: !!el('report-identity')?.checked });
  /** The shared complex-action contract, one entry per draft (issue #1510).
   *
   * Dirty is what this panel holds and has not sent. `keepReusable` is the
   * shared rule Spawn set: after an authoritative success the choices that
   * describe WHAT stay, and the ones that describe WHICH ONE and WHERE go, so
   * a second false report of the same kind is one field away. */
  const drafts = {
    misclassify: {
      isDirty: () => !!el('classification')?.value,
      reset: ({ keepReusable = false } = {}) => {
        if (!keepReusable && el('classification')) el('classification').value = '';
        render();
      },
      keepOpen: () => keepOpen('misclassify'),
      focus: () => el('classification')?.focus(),
    },
    'report-policy': {
      isDirty: () => validPolicy(chosenPolicy()),
      reset: ({ keepReusable = false } = {}) => {
        if (!keepReusable) {
          if (el('report-delay')) el('report-delay').value = '0';
          if (el('report-step')) el('report-step').value = '0';
          if (el('report-identity')) el('report-identity').checked = false;
        }
        render();
      },
      keepOpen: () => keepOpen('report'),
      focus: () => el('report-delay')?.focus(),
    },
  };
  function render() {
    if (el('target')) el('target').textContent = selected ? wireText(selected.name) : t('server.gm.contact.select');
    // Each draft is a panel of its own, so it names the observer and target it
    // would act on rather than relying on the tool that used to hold it.
    const ship = entities.find(row => row.entity_id === observer);
    const scope = observer && selected
      ? t('server.gm.contact.draft_scope', { ship: wireText(ship?.name || observer), target: wireText(selected.name) })
      : t('server.gm.contact.select');
    for (const draft of ['misclassify', 'report']) {
      if (el(`${draft}-scope`)) el(`${draft}-scope`).textContent = scope;
    }
    if (el('mode')) el('mode').textContent = t(`server.gm.contact.${overrides[observer]?.[selected?.entity_id] || 'normal'}`);
    for (const mode of MODES) if (el(mode)) el(mode).disabled = !valid();
    const current = classifications[observer]?.[selected?.entity_id];
    if (el('classification-current')) el('classification-current').textContent = current ? wireText(current.label) : t('server.gm.contact.classification-normal');
    if (el('classification')) el('classification').disabled = !valid() || !palette.length;
    if (el('misclassify')) el('misclassify').disabled = !valid() || !palette.some(row => row.palette === el('classification')?.value);
    if (el('classification-normal')) el('classification-normal').disabled = !valid() || !current;
    const report = information.reports?.[observer]?.[selected?.entity_id];
    for (const id of ['report-delay', 'report-step', 'report-identity']) if (el(id)) el(id).disabled = !valid();
    if (el('report-set')) el('report-set').disabled = !valid() || !validPolicy(chosenPolicy());
    if (el('report-clear')) el('report-clear').disabled = !valid() || !report;
    if (el('report-current')) el('report-current').textContent = report ? t('server.gm.contact.report_current', { delay: report.policy.delay_ticks, step: report.policy.position_step_mm, identity: t(report.policy.hide_identity ? 'server.gm.contact.report_hidden' : 'server.gm.contact.report_visible') }) : t('server.gm.contact.report_normal');
    const ghostKey = JSON.stringify([observer, information.ghosts?.[observer], !!validObserver()]);
    if (el('ghosts') && renderedGhosts !== ghostKey) {
      renderedGhosts = ghostKey;
      el('ghosts').replaceChildren();
      // Placing a ghost is a Spawn outcome since the ghost draft was retired
      // (Live layout version 14), so this list is what remains on the contact
      // tool: the record of what each observing ship has been told, and the
      // one simple action that undoes it. Removal is a verb, not a draft.
      for (const ghost of Object.values(information.ghosts?.[observer] || {})) {
        const li = doc.createElement('li');
        const label = doc.createElement('span');
        label.textContent = `${ghost.id}: ${wireText(ghost.label)} (${ghost.position_mm.join(', ')})`;
        const remove = doc.createElement('button'); remove.type = 'button';
        remove.dataset.role = 'remove'; remove.dataset.ghostId = ghost.id;
        remove.textContent = t('server.gm.contact.ghost-remove');
        remove.setAttribute('aria-label', t('server.gm.contact.ghost-remove_accessibility', { id: ghost.id }));
        remove.disabled = !validObserver();
        remove.addEventListener('click', () => chooseInformation({ remove_ghost: { id: ghost.id } }));
        li.append(label, remove); el('ghosts').appendChild(li);
      }
    }
  }
  /** Which draft a request finishes, or null when it is a simple action.
   * Normal classification, clearing a policy and removing a ghost each undo
   * something rather than compose it, so they finish no draft. */
  function draftOf(request) {
    if (!request) return null;
    if (Object.hasOwn(request, 'palette')) return request.palette === null ? null : 'misclassify';
    if (request.change?.set_report_policy) return 'report-policy';
    return null;
  }
  /** Say how a request went ON THE PANEL IT WAS COMPOSED ON.
   *
   * Since issue #1510 the three drafts are panels of their own, and a panel
   * that is not the active tab is `hidden`: a refusal written only into the
   * contact tool would be a refusal the operator never sees. The contact tool
   * keeps the log of every result, so it is told as well. */
  function feedback(state, request = pending) {
    const draft = draftOf(request);
    const nodes = [el('feedback'), draft && el(`${draft === 'report-policy' ? 'report' : draft}-feedback`)];
    for (const node of nodes) {
      if (!node) continue;
      node.dataset.state = state;
      node.textContent = t(`server.gm.contact.${state}`);
    }
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
  function chooseClassification(choice) {
    if (!valid() || (choice !== null && !palette.some(row => row.palette === choice))) return false;
    const chosen = { operator_id: getOperator().id, ship: observer, target: selected.entity_id, palette: choice };
    const description = t('settings.gm.confirmation.contact', {
      mode: choice === null ? t('server.gm.contact.classification-normal') : wireText(palette.find(row => row.palette === choice).label),
      target: wireText(selected.name), ship: wireText(entities.find(row => row.entity_id === observer)?.name || observer),
    });
    return confirmAction({ category: 'contact.override', description, preview: () => description,
      accept: () => submitChosen(chosen) });
  }
  function chooseInformation(change) {
    const target = informationTarget(change);
    if (!validObserver() || !boundedId(target)) return false;
    const policy = change.set_report_policy;
    if (policy && (!valid() || policy.target !== selected.entity_id || !validPolicy(policy.policy))) return false;
    if (change.clear_report_policy && (!valid() || change.clear_report_policy.target !== selected.entity_id)) return false;
    if (!change.remove_ghost && !policy && !change.clear_report_policy) return false;
    const chosen = { operator_id: getOperator().id, ship: observer, change: structuredClone(change) };
    const description = t('settings.gm.confirmation.contact', { mode: t(policy ? 'server.gm.contact.report_set' : change.clear_report_policy ? 'server.gm.contact.report_normal' : 'server.gm.contact.ghost-remove'), target,
      ship: wireText(entities.find(row => row.entity_id === observer)?.name || observer) });
    return confirmAction({ category: 'contact.override', description, preview: () => description, accept: () => submitChosen(chosen) });
  }
  function submitChosen(chosen) {
    if (pending || getOperator()?.id !== chosen.operator_id) return false;
    const request = { ...chosen, correlation: correlation() };
    let accepted = false;
    try { accepted = (request.change ? submitInformation(request) : Object.hasOwn(request, 'palette') ? submitClassification(request) : submit(request)) !== false; } catch (_) { /* report below */ }
    // Whatever the last request left on another draft is old news now —
    // including when this one is refused before it ever leaves the desk.
    for (const node of ['misclassify-feedback', 'report-feedback']) {
      if (el(node)) { el(node).textContent = ''; delete el(node).dataset.state; }
    }
    if (!accepted) { feedback('refused', chosen); return false; }
    pending = request;
    feedback('pending');
    timer = schedule(() => { pending = null; timer = null; feedback('timed_out', request); render(); }, DEFAULT_ACTION_FEEDBACK_TIMEOUT_MS);
    render(); return true;
  }
  function update(payload) {
    let value = payload;
    if (typeof value === 'string') { try { value = JSON.parse(value); } catch (_) { return false; } }
    if (!value || !Array.isArray(value.entities) || !Array.isArray(value.contact_results || [])) return false;
    const rows = value.contact_results || [];
    if (rows.some(row => !row || !RESULT_MODES.some(mode => row.action_kind === `contact-${mode}`)
      || typeof row.observer !== 'string' || !row.observer || typeof row.target !== 'string' || !row.target
      || typeof row.operator_id !== 'string' || !row.operator_id || typeof row.correlation !== 'string' || !row.correlation
      || !Number.isSafeInteger(row.tick) || !['applied', 'no-op', 'refused'].includes(row.outcome))) return false;
    const nextOverrides = value.contact_overrides || {};
    if (typeof nextOverrides !== 'object' || Object.values(nextOverrides).some(targets => !targets || typeof targets !== 'object'
      || Object.values(targets).some(mode => !MODES.includes(mode)))) return false;
    const nextClassifications = value.contact_classifications || {};
    const nextPalette = value.contact_classification_palette || [];
    const reported = row => row && typeof row.palette === 'string' && row.palette && typeof row.label === 'string' && row.label;
    if (!Array.isArray(nextPalette) || nextPalette.some(row => !reported(row))
      || new Set(nextPalette.map(row => row.palette)).size !== nextPalette.length
      || !nextClassifications || typeof nextClassifications !== 'object' || Array.isArray(nextClassifications)
      || Object.values(nextClassifications).some(targets => !targets || typeof targets !== 'object' || Array.isArray(targets)
        || Object.values(targets).some(row => !reported(row)))) return false;
    const nextInformation = value.contact_information || { ghosts: {} };
    if (!nextInformation || typeof nextInformation !== 'object' || !nextInformation.ghosts || typeof nextInformation.ghosts !== 'object'
      || Object.values(nextInformation.ghosts).some(rows => !rows || typeof rows !== 'object' || Object.entries(rows).some(([id, ghost]) => !ghost || ghost.id !== id || !boundedId(id) || !reported(ghost) || !Array.isArray(ghost.position_mm) || ghost.position_mm.length !== 3 || !validPosition(ghost.position_mm)))) return false;
    if (nextInformation.reports && (typeof nextInformation.reports !== "object" || Array.isArray(nextInformation.reports)
      || Object.values(nextInformation.reports).some(rows => !rows || typeof rows !== "object" || Array.isArray(rows) || Object.values(rows).some(row => !row || !validPolicy(row.policy))))) return false;
    information = nextInformation;
    entities = value.entities; overrides = nextOverrides; classifications = nextClassifications; palette = nextPalette;
    // Moving entities publish continuously. Keep the native dropdown's option
    // nodes intact while an operator is choosing from an unchanged palette.
    const paletteKey = JSON.stringify(palette.map(row => [row.palette, row.label]));
    if (renderedPalette !== paletteKey) for (const classificationSelect of [el('classification')].filter(Boolean)) {
      const prior = classificationSelect.value;
      classificationSelect.replaceChildren();
      const placeholder = doc.createElement('option'); placeholder.value = ''; placeholder.textContent = t('server.gm.contact.classification'); classificationSelect.appendChild(placeholder);
      for (const row of palette) {
        const option = doc.createElement('option'); option.value = row.palette; option.textContent = wireText(row.label); classificationSelect.appendChild(option);
      }
      classificationSelect.value = palette.some(row => row.palette === prior) ? prior : '';
    }
    renderedPalette = paletteKey;
    const select = el('observer');
    const observerKey = JSON.stringify(entities.filter(row => row.kind === 'player_ship').map(row => [row.entity_id, row.name]));
    if (select && renderedObservers !== observerKey) {
      renderedObservers = observerKey;
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
      && row.observer === pending.ship && row.target === (pending.change ? informationTarget(pending.change) : pending.target) && row.action_kind === requestedKind(pending));
    if (terminal) {
      if (timer !== null) cancelSchedule(timer);
      timer = null;
      const settled = pending;
      pending = null;
      feedback(terminal.outcome === 'refused' ? 'refused' : 'applied', settled);
      // Only a change the world took finishes a draft. A refusal and a no-op
      // both leave the operator with something to answer, on the panel that
      // holds what they typed.
      if (terminal.outcome === 'applied') {
        const draft = draftOf(settled);
        if (draft) onSucceeded(draft);
      }
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
    timer = null; pending = null; selected = null; observer = ''; entities = []; overrides = {}; classifications = {}; palette = [];
    renderedPalette = null; renderedObservers = null; renderedGhosts = null; information = { ghosts: {} };
    el('results')?.replaceChildren();
    for (const node of ['feedback', 'misclassify-feedback', 'report-feedback']) {
      if (el(node)) { el(node).textContent = ''; delete el(node).dataset.state; }
    }
    // The run these drafts were for has gone, so what was typed into them has
    // gone with it — including a draft nobody had open, which no lifecycle
    // discard would ever reach.
    for (const draft of Object.values(drafts)) draft.reset({ keepReusable: false });
    render();
  }
  el('observer')?.addEventListener('change', () => { observer = el('observer').value; render(); });
  for (const mode of MODES) el(mode)?.addEventListener('click', () => choose(mode));
  el('classification')?.addEventListener('change', render);
  el('misclassify')?.addEventListener('click', () => chooseClassification(el('classification').value));
  el('classification-normal')?.addEventListener('click', () => chooseClassification(null));
  for (const id of ['report-delay', 'report-step', 'report-identity']) el(id)?.addEventListener('input', render);
  el('report-set')?.addEventListener('click', () => chooseInformation({ set_report_policy: { target: selected?.entity_id, policy: chosenPolicy() } }));
  el('report-clear')?.addEventListener('click', () => chooseInformation({ clear_report_policy: { target: selected?.entity_id } }));
  render();
  return { update, choose, chooseClassification, chooseInformation, reset, refreshAdmission: render, select: entity => { selected = entity; render(); },
    dispose() {},
    drafts,
    state: () => ({ observer, target: selected?.entity_id || null, pending, overrides, classifications, palette, information }) };
}
