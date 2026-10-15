/**
 * gui/scenario-state-panel.js — the debug dock's scenario-state panel
 * (issue #1148, PRD #1144).
 *
 * The second JSON-driven renderer on the structured debug pipeline. It parses
 * the `ScenarioStatePayload` the WASM bridge publishes (`wasm_get_scenario_state`)
 * and lays out the running scenario's working state — flags, objectives,
 * triggers with eligibility, the delayed-action and deadline queues, the
 * commitments board and the comms dossier — so a scenario author can answer "why
 * didn't the story beat fire?" without a snapshot or a digest hash. Nothing here
 * talks to the simulation: it is a pure function of the payload, unit-tested in
 * jsdom without a browser or a WASM bundle, exactly like
 * `gui/station-activity-chart.js`.
 *
 * It copies that renderer's pattern: a `parse*` guard, a pure `build*(payload)`
 * that returns DOM, and a `render*(container, json)` wrapper the settings cog
 * wires in.
 */

import { t } from './strings.js';

/**
 * Parse the raw bridge JSON into a payload, or `null` when there is nothing
 * renderable yet (empty string before the first publish, or malformed input).
 *
 * @param {string} json
 * @returns {object|null}
 */
export function parseScenarioState(json) {
  if (typeof json !== 'string' || json.length === 0) return null;
  let payload;
  try {
    payload = JSON.parse(json);
  } catch {
    return null;
  }
  if (!payload || typeof payload !== 'object') return null;
  // The seven surfaces are always-present arrays on a real payload; a missing
  // one means this is not a scenario-state payload.
  if (!Array.isArray(payload.flags) || !Array.isArray(payload.triggers)) return null;
  return payload;
}

/** Whether every surface on the payload is empty — the "no scenario" state. */
function isEmptyPayload(payload) {
  if (payload.planning && typeof payload.planning === 'object') return false;
  return [
    payload.flags,
    payload.objectives,
    payload.triggers,
    payload.delayed_actions,
    payload.deadlines,
    payload.commitments,
    payload.dossier,
    payload.situations,
  ].every((list) => !Array.isArray(list) || list.length === 0);
}

function el(doc, tag, className, text) {
  const node = doc.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

/** A section wrapper with a localised heading and a `data-section` handle. */
function section(doc, id, labelId) {
  const wrap = el(doc, 'div', 'ss-section');
  wrap.setAttribute('data-section', id);
  wrap.appendChild(el(doc, 'div', 'ss-section-title', t(labelId)));
  return wrap;
}

/** A muted "(none)" line for an empty surface, so the author sees it exists. */
function noneRow(doc) {
  return el(doc, 'div', 'ss-none', t('settings.debug.scenario.none'));
}

/**
 * Build the panel DOM from a parsed payload. Pure: returns a detached element,
 * mutates nothing.
 *
 * @param {object} payload  a parsed `ScenarioStatePayload`
 * @param {{doc?: Document}} [opts]
 * @returns {HTMLElement}
 */
export function buildScenarioStatePanel(payload, opts = {}) {
  const doc = opts.doc || document;
  const root = el(doc, 'div', 'ss-panel');
  root.appendChild(el(doc, 'div', 'ss-title', t('settings.debug.scenario')));

  // ── Flags ──
  {
    const sec = section(doc, 'flags', 'settings.debug.scenario.flags');
    const flags = payload.flags || [];
    if (flags.length === 0) sec.appendChild(noneRow(doc));
    for (const flag of flags) {
      const row = el(doc, 'div', 'ss-flag');
      row.setAttribute('data-flag', String(flag.name));
      row.appendChild(el(doc, 'span', 'ss-flag-name', String(flag.name)));
      row.appendChild(el(doc, 'span', 'ss-flag-value', String(flag.value)));
      sec.appendChild(row);
    }
    root.appendChild(sec);
  }

  // ── Objectives ──
  {
    const sec = section(doc, 'objectives', 'settings.debug.scenario.objectives');
    const objectives = payload.objectives || [];
    if (objectives.length === 0) sec.appendChild(noneRow(doc));
    for (const obj of objectives) {
      const row = el(doc, 'div', 'ss-objective');
      row.setAttribute('data-id', String(obj.id));
      row.setAttribute('data-status', String(obj.status));
      if (obj.mandatory) row.setAttribute('data-mandatory', 'true');
      row.appendChild(el(doc, 'span', 'ss-objective-id', String(obj.id)));
      row.appendChild(el(doc, 'span', 'ss-objective-status', String(obj.status)));
      row.appendChild(
        el(doc, 'span', 'ss-objective-priority', String(obj.base_priority)),
      );
      const directive = obj.directive && obj.directive.kind ? obj.directive.kind : 'None';
      const directiveEl = el(doc, 'span', 'ss-objective-directive', String(directive));
      directiveEl.setAttribute('data-directive', String(directive));
      row.appendChild(directiveEl);
      sec.appendChild(row);
    }
    root.appendChild(sec);
  }

  // ── Triggers ──
  {
    const sec = section(doc, 'triggers', 'settings.debug.scenario.triggers');
    const triggers = payload.triggers || [];
    if (triggers.length === 0) sec.appendChild(noneRow(doc));
    for (const trig of triggers) {
      const row = el(doc, 'div', 'ss-trigger');
      if (trig.id) row.setAttribute('data-id', String(trig.id));
      row.setAttribute('data-pending', trig.pending ? 'true' : 'false');
      row.setAttribute('data-fired', trig.fired ? 'true' : 'false');
      row.setAttribute('data-when-holds', trig.when_holds ? 'true' : 'false');
      if (trig.repeat) row.setAttribute('data-repeat', 'true');
      row.appendChild(el(doc, 'span', 'ss-trigger-condition', String(trig.condition)));
      if (trig.when) {
        const whenEl = el(doc, 'span', 'ss-trigger-when', String(trig.when));
        // The eligibility signal the surface exists for: an armed beat whose
        // gate is not holding is waiting, not dead.
        whenEl.setAttribute('data-holds', trig.when_holds ? 'true' : 'false');
        row.appendChild(whenEl);
      }
      // A compact status badge: fired / pending / waiting-on-gate.
      let statusId;
      if (trig.fired && !trig.repeat) statusId = 'settings.debug.scenario.trigger_fired';
      else if (!trig.when_holds) statusId = 'settings.debug.scenario.trigger_waiting';
      else statusId = 'settings.debug.scenario.trigger_armed';
      const badge = el(doc, 'span', 'ss-trigger-status', t(statusId));
      badge.setAttribute(
        'data-state',
        trig.fired && !trig.repeat ? 'fired' : trig.when_holds ? 'armed' : 'waiting',
      );
      row.appendChild(badge);

      // ── Fire history (#1151) ──
      // The bounded record of this trigger's recent fires with the predicate
      // values observed at each — how an author reconstructs why a beat fired
      // early, late, or not at all. Absent on a pre-#1151 payload, and empty
      // until the trigger fires while capture is on: shown only when non-empty so
      // a never-fired trigger stays uncluttered.
      const fires = Array.isArray(trig.fire_history) ? trig.fire_history : [];
      if (fires.length > 0) {
        const hist = el(doc, 'div', 'ss-trigger-fires');
        hist.setAttribute('data-count', String(fires.length));
        for (const fire of fires) {
          const fireRow = el(doc, 'div', 'ss-fire');
          fireRow.setAttribute('data-fired-secs', String(fire.fired_secs));
          fireRow.appendChild(
            el(
              doc,
              'span',
              'ss-fire-when',
              t('settings.debug.scenario.at_secs', { secs: fire.fired_secs }),
            ),
          );
          const values = Array.isArray(fire.predicate_values) ? fire.predicate_values : [];
          for (const pv of values) {
            const pvEl = el(doc, 'span', 'ss-fire-value');
            pvEl.setAttribute('data-atom', String(pv.atom));
            pvEl.textContent = `${pv.atom} = ${pv.value}`;
            fireRow.appendChild(pvEl);
          }
          hist.appendChild(fireRow);
        }
        row.appendChild(hist);
      }
      sec.appendChild(row);
    }
    root.appendChild(sec);
  }

  // ── Delayed actions ──
  {
    const sec = section(doc, 'delayed', 'settings.debug.scenario.delayed');
    const actions = payload.delayed_actions || [];
    if (actions.length === 0) sec.appendChild(noneRow(doc));
    for (const act of actions) {
      const row = el(doc, 'div', 'ss-delayed');
      row.appendChild(el(doc, 'span', 'ss-delayed-action', String(act.action)));
      if (act.entity) row.appendChild(el(doc, 'span', 'ss-delayed-entity', String(act.entity)));
      row.appendChild(
        el(
          doc,
          'span',
          'ss-delayed-when',
          t('settings.debug.scenario.at_secs', { secs: act.fire_at_secs }),
        ),
      );
      sec.appendChild(row);
    }
    root.appendChild(sec);
  }

  // ── Deadlines ──
  {
    const sec = section(doc, 'deadlines', 'settings.debug.scenario.deadlines');
    const deadlines = payload.deadlines || [];
    if (deadlines.length === 0) sec.appendChild(noneRow(doc));
    for (const dl of deadlines) {
      const row = el(doc, 'div', 'ss-deadline');
      row.setAttribute('data-id', String(dl.id));
      row.setAttribute('data-state', String(dl.state));
      if (dl.visible) row.setAttribute('data-visible', 'true');
      row.appendChild(el(doc, 'span', 'ss-deadline-id', String(dl.id)));
      row.appendChild(el(doc, 'span', 'ss-deadline-state', String(dl.state)));
      row.appendChild(el(doc, 'span', 'ss-deadline-due', String(dl.due_tick)));
      sec.appendChild(row);
    }
    root.appendChild(sec);
  }

  // ── Commitments ──
  {
    const sec = section(doc, 'commitments', 'settings.debug.scenario.commitments');
    const commitments = payload.commitments || [];
    if (commitments.length === 0) sec.appendChild(noneRow(doc));
    for (const c of commitments) {
      const row = el(doc, 'div', 'ss-commitment');
      row.setAttribute('data-id', String(c.id));
      row.setAttribute('data-state', String(c.state));
      row.appendChild(el(doc, 'span', 'ss-commitment-id', String(c.id)));
      row.appendChild(el(doc, 'span', 'ss-commitment-made-to', String(c.made_to)));
      row.appendChild(el(doc, 'span', 'ss-commitment-state', String(c.state)));
      sec.appendChild(row);
    }
    root.appendChild(sec);
  }

  // ── Dossier ──
  {
    const sec = section(doc, 'dossier', 'settings.debug.scenario.dossier');
    const dossier = payload.dossier || [];
    if (dossier.length === 0) sec.appendChild(noneRow(doc));
    for (const entry of dossier) {
      const row = el(doc, 'div', 'ss-dossier-entry');
      row.setAttribute('data-provenance', String(entry.provenance));
      row.appendChild(el(doc, 'span', 'ss-dossier-subject', String(entry.subject_uuid)));
      row.appendChild(el(doc, 'span', 'ss-dossier-text', String(entry.text)));
      row.appendChild(el(doc, 'span', 'ss-dossier-provenance', String(entry.provenance)));
      sec.appendChild(row);
    }
    root.appendChild(sec);
  }

  // Author-only derived graph; it cannot issue instructions or reinterpret
  // original capability explanations as current machinery/stock information.
  if (payload.planning && typeof payload.planning === 'object') {
    const plan = payload.planning, choice = plan.choice;
    const sec = section(doc, 'planning', 'settings.debug.scenario.planning.title');
    const field = (key, value, className=key) => {
      const row = el(doc, 'div', `ss-plan-${className}`);
      row.appendChild(el(doc, 'span', '', t(`settings.debug.scenario.planning.${key}`)+': '));
      row.appendChild(el(doc, 'span', '', value == null ? t('settings.debug.scenario.none') : String(value)));
      sec.appendChild(row);
    };
    field('root', plan.source?.task);
    field('chosen', choice?.chosen_tick);
    field('actor', choice?.actor_uuid);
    for (const [node, method] of Object.entries(choice?.methods || {})) {
      field('method', `${node}: ${method.task?.library_path}::${method.task?.task} / ${method.method}`);
    }
    for (const reason of Array.isArray(choice?.refused_methods) ? choice.refused_methods : []) field('refused', reason);
    const revisions = Array.isArray(choice?.temporal_reconsiderations) ? choice.temporal_reconsiderations : [];
    if (revisions.length) {
      field('original_schedule', choice?.temporal?.outcome?.state);
      for (const revision of revisions) {
        field('reconsideration', `${revision.tick}: ${revision.cause?.kind} / ${(revision.cause?.owners || []).join(', ')} / ${revision.decision?.outcome?.state}`);
      }
    }
    const temporal = revisions.at(-1)?.decision || choice?.temporal;
    if (temporal && typeof temporal === 'object') {
      const outcome = temporal.outcome;
      field('schedule', outcome?.state);
      field('profile', temporal.profile_index);
      field('deadline', temporal.deadline_tick);
      if (outcome?.state === 'refused') field('schedule_refused', outcome.reason);
      if (outcome?.state === 'committed') {
        for (const [node, interval] of Object.entries(outcome.nodes || {})) {
          field('interval', `${node}: ${interval.start_tick} → ${interval.end_tick}`);
        }
        if (Number.isSafeInteger(plan.observed_tick) && Number.isSafeInteger(temporal.deadline_tick)
            && plan.observed_tick > temporal.deadline_tick) field('overdue', plan.observed_tick);
      }
      for (const [owner, input] of Object.entries(temporal.input?.owners || {})) {
        field('operator', `${owner}: ${input.source}${input.station ? ' / '+input.station+' / '+input.rating : ''}${input.reason ? ' / '+input.reason : ''}`);
      }
      for (const bucket of Array.isArray(plan.workload) ? plan.workload : []) {
        field('workload', `${bucket.station}: ${bucket.minute} / ${bucket.attention_nanos}`);
      }
    }
    for (const leaf of Array.isArray(plan.leaves) ? plan.leaves : []) {
      field('leaf', `${leaf.node}: ${leaf.instance}`);
      field('released', leaf.released_tick);
    }
    for (const edge of Array.isArray(plan.dependencies) ? plan.dependencies : []) {
      field('dependency', `${edge.from} → ${edge.to}: ${edge.when?.kind}${edge.when?.outcome ? ' / '+edge.when.outcome : ''}`);
    }
    if (plan.derivation_error) field('error', plan.derivation_error);
    root.appendChild(sec);
  }

  // Same canonical lifecycle; pending facts remain visibly distinct from
  // acknowledged achievement/settlement and never imply a completed job.
  {
    const sec = section(doc, 'situations', 'settings.debug.scenario.situations');
    const situations = Array.isArray(payload.situations) ? payload.situations : [];
    if (situations.length === 0) sec.appendChild(noneRow(doc));
    for (const {id, lifecycle: state, pending = []} of situations) {
      if (!state || typeof state !== 'object') continue;
      const row = el(doc, 'div', 'ss-situation');
      row.dataset.id = String(id);
      row.appendChild(el(doc, 'strong', 'ss-situation-id', String(id)));
      const fields = [
        ['definition', state.definition], ['request', JSON.stringify(state.request)],
        ['armed', state.armed_tick], ['job', state.accepted?.identity?.kind === 'physical' ? state.accepted.identity.job : null],
        ['transmission', state.accepted?.identity?.kind === 'communication' ? state.accepted.identity.origin_id : null],
        ['recipient', state.accepted?.identity?.kind === 'communication' ? state.accepted.identity.recipient : null],
        ['accepted', state.accepted_tick], ['achieved', state.achieved?.tick],
        ['settled', state.settled?.tick], ['outcome', state.settled ? JSON.stringify(state.settled.outcome) : null],
        ['state', state.state],
      ];
      for (const [key, value] of fields) {
        const field = el(doc, 'div', `ss-situation-${key}`);
        field.appendChild(el(doc, 'span', '', t(`settings.debug.scenario.activity.${key}`) + ': '));
        field.appendChild(el(doc, 'span', '', value == null ? t('settings.debug.scenario.none') : String(value)));
        row.appendChild(field);
      }
      for (const fact of Array.isArray(pending) ? pending : []) {
        row.appendChild(el(doc, 'div', 'ss-situation-pending', t('settings.debug.scenario.activity.pending',
          {tick:fact.tick, kind:fact.outcome?.kind})));
      }
      for (const [stateId, result] of Object.entries(state.activations || {})) {
        row.appendChild(el(doc, 'div', 'ss-situation-activation', `${stateId}: ${result}${state.activation_ticks?.[stateId] == null ? '' : ' @ ' + state.activation_ticks[stateId]}`));
      }
      sec.appendChild(row);
    }
    root.appendChild(sec);
  }
  return root;
}

/**
 * Render the panel (or an empty-state placeholder) into `container` from the raw
 * bridge JSON. Clears the container first. The settings cog calls this each
 * frame while the scenario-state output is the visible one.
 *
 * @param {Element} container
 * @param {string} json  raw JSON from `wasm_get_scenario_state()`
 * @param {{doc?: Document}} [opts]
 */
export function renderScenarioStatePanel(container, json, opts = {}) {
  if (!container) return;
  const doc = container.ownerDocument || opts.doc || document;
  const payload = parseScenarioState(json);
  container.textContent = '';
  if (!payload || isEmptyPayload(payload)) {
    const empty = el(doc, 'div', 'ss-empty', t('settings.debug.scenario_empty'));
    container.appendChild(empty);
    return;
  }
  container.appendChild(buildScenarioStatePanel(payload, { doc }));
}

// Expose for the classic-script bootstrap in server.html, which wires this
// renderer into the settings cog's scenario-state output.
if (typeof window !== 'undefined') {
  window.renderScenarioStatePanel = renderScenarioStatePanel;
}
