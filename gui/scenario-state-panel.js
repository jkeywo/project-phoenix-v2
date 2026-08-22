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
  return [
    payload.flags,
    payload.objectives,
    payload.triggers,
    payload.delayed_actions,
    payload.deadlines,
    payload.commitments,
    payload.dossier,
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
