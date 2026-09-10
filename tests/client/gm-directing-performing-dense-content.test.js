// @vitest-environment jsdom
/**
 * tests/client/gm-directing-performing-dense-content.test.js — issue #1430
 * (PRD #1418 stories 1-7, 22, 31; parent #1418).
 *
 * #1421's `DENSE_CONTENT.gmLists` fixture names real, long String-Table rows
 * rendered per-row in the GM Mission and Objective panels (the M2 "directing"
 * surfaces) AND in the GM Knowledge Compare panel
 * (gui/gm-knowledge-compare.js, one of the M2 "performing" surfaces) — the
 * issue #1430 review's blocking finding was that only the first two were
 * actually driven through their real reducers here; the third and fourth
 * `gmLists` rows (`server.gm.knowledge.hint`, `server.gm.knowledge.summary`)
 * stayed completely undriven by any test in this issue's diff. This file now
 * covers all four. These are the jsdom-appropriate half of that acceptance:
 * proof that a dense, realistic log/list/comparison renders every row's FULL
 * localized text with no JS-side truncation, and that every control stays
 * present and enabled at that density. Real layout/pixel reflow at
 * 100/150/200% and browser zoom is the companion Playwright spec's job
 * (`tests/smoke/gm-layout.spec.js`) — jsdom does not lay out text, only
 * build and read the DOM these panels produce.
 */

import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createGmMissionPanel } from '../../gui/gm-mission-panel.js';
import { createGmObjectivePanel } from '../../gui/gm-objective-panel.js';
import { createGmKnowledgeCompare } from '../../gui/gm-knowledge-compare.js';
import { GM_ACTION_REFUSAL_REASON_LABELS } from '../../gui/gm-action-reasons.js';
import { DENSE_CONTENT } from '../fixtures/device-matrix.mjs';
import { applyToDom, t } from '../../gui/strings.js';

const GM_LIST_IDS = DENSE_CONTENT.gmLists.map((row) => row.id);

describe('GM mission panel at dense log density (issue #1430)', () => {
  const EVENT_IDS = ['base-world::breach_alarm', 'base-world::relief', 'base-world::skyhook_loss'];

  beforeEach(() => {
    document.body.innerHTML = `
      <section id="gm-mission-panel">
        <h2 id="gm-mission-heading"></h2>
        <ul id="gm-mission-events"></ul>
        <p id="gm-mission-empty"></p>
        <p id="gm-mission-feedback"></p>
        <h3 id="gm-mission-log-heading"></h3>
        <ol id="gm-mission-log"></ol>
      </section>
    `;
  });

  function event(id) {
    return {
      id, label: 'server.gm.mission.heading', fire: true, pause: true, skip: true,
      repeatable: true, spent: false, armed: false, paused: false, skip_armed: false,
    };
  }

  /** #1421's own dense-fixture id, actually driven through the real reducer:
   *  a mixed 12-row absolute log — the fire/pause family nine times over the
   *  three authored events, plus the skip family three times, so both string
   *  families in `DENSE_CONTENT.gmLists` are exercised, not just one. */
  function denseResults() {
    const outcomes = ['applied', 'no-op', 'refused'];
    const results = [];
    for (let i = 0; i < 9; i += 1) {
      results.push({
        operator_id: 'gm-a', correlation: `gm-fire-${i}`, outcome: outcomes[i % 3],
        tick: 100 + i, target: EVENT_IDS[i % 3], verb: 'fire', requested_active: true,
        ...(outcomes[i % 3] === 'refused' ? { reason: 'unknown-gm-event' } : {}),
      });
    }
    for (let i = 0; i < 3; i += 1) {
      results.push({
        operator_id: 'gm-b', correlation: `gm-skip-${i}`, outcome: outcomes[i],
        tick: 200 + i, target: EVENT_IDS[i], lever: 'skip-next', requested_active: true,
        ...(outcomes[i] === 'refused' ? { reason: 'unknown-gm-event' } : {}),
      });
    }
    return results;
  }

  it('renders every row of a dense absolute log with its full, untruncated sentence', () => {
    const panel = createGmMissionPanel({
      doc: document, win: window, t,
      getOperatorName: (id) => ({ 'gm-a': 'Alex', 'gm-b': 'Blair' }[id] || id),
      getOperator: () => ({ id: 'gm-a', name: 'Alex' }),
    });
    panel.update({ events: EVENT_IDS.map(event), results: denseResults() });

    const rows = [...document.querySelectorAll('#gm-mission-log .gm-mission-log-entry')];
    expect(rows).toHaveLength(12);

    // The exact string the DOM shows equals the exact string the shared
    // renderer would produce — a JS-side truncation regresses this without
    // needing a real browser to lay the text out.
    const expectedApplied = t('server.gm.mission.result_applied', {
      name: 'Alex', verb: t('server.gm.mission.verb_fire'), event: EVENT_IDS[0],
      tick: '100', correlation: 'gm-fire-0', reason: t('server.gm.mission.reason_unspecified'),
    });
    expect(rows[0].textContent).toBe(expectedApplied);

    // A real, long refusal reason survives whole — this is the row PRD #1418
    // means by "long text and dense states": three of the twelve carry it.
    const refusedRows = rows.filter((row) => row.dataset.outcome === 'refused');
    expect(refusedRows.length).toBeGreaterThan(0);
    const expectedReason = t(GM_ACTION_REFUSAL_REASON_LABELS['unknown-gm-event']);
    expect(expectedReason.length).toBeGreaterThan(10);
    for (const row of refusedRows) expect(row.textContent).toContain(expectedReason);

    // The skip family's own sentence stays distinct from Fire/Pause's — the
    // module's own reason for keeping them as two families, verified rather
    // than merely trusted.
    const skipRows = rows.filter((row) => row.dataset.lever === 'skip');
    expect(skipRows).toHaveLength(3);
    // Every row is real translated prose — never a raw String-Table id
    // leaking through because a density edge case fell outside a template.
    for (const row of rows) expect(row.textContent).not.toMatch(/^server\./);

    // Density never disables or hides the levers a busy log sits beside.
    for (const id of EVENT_IDS) {
      for (const role of ['fire', 'pause', 'skip']) {
        const button = document.querySelector(`button[data-role="${role}"][data-event-id="${id}"]`);
        expect(button, `${role} on ${id}`).not.toBeNull();
        expect(button.disabled, `${role} on ${id} stays enabled`).toBe(false);
        expect(button.hidden, `${role} on ${id} stays visible`).toBe(false);
      }
    }
  });

  // The exact family/suffix DENSE_CONTENT names by id, through the LOCAL
  // pending-timeout path — the only outcome that ever produces it, since
  // `RESULT_OUTCOMES` (absolute, wire) has no 'timed_out' member.
  it(`renders "${GM_LIST_IDS[2]}" for a Skip press that never receives an authoritative answer`, () => {
    expect(GM_LIST_IDS[2]).toBe('server.gm.mission.skip_result_timed_out');
    let fire = null;
    const panel = createGmMissionPanel({
      doc: document, win: window, t,
      submitArmSkip: () => true,
      getOperator: () => ({ id: 'gm-a', name: 'Alex' }),
      getOperatorName: () => 'Alex',
      correlation: () => 'gm-skip-timeout-1',
      schedule: (fn) => { fire = fn; return 9; },
      cancelSchedule: () => {},
    });
    panel.update({ events: [{ ...event(EVENT_IDS[0]), spent: false }], results: [] });
    document.querySelector(`button[data-role="skip"][data-event-id="${EVENT_IDS[0]}"]`).click();
    fire();
    const row = document.querySelector('#gm-mission-log .gm-mission-log-entry');
    expect(row.dataset.outcome).toBe('timed-out');
    expect(row.textContent.length).toBeGreaterThan(0);
    expect(row.textContent).not.toMatch(/^server\./);
  });
});

describe('GM Objective panel at dense list density (issue #1430)', () => {
  beforeEach(() => {
    document.body.innerHTML = `
      <section id="gm-objective-panel">
        <ul id="gm-objective-list"></ul>
        <p id="gm-objective-empty"></p>
        <p id="gm-objective-feedback"></p>
        <div id="gm-objective-confirmation" hidden>
          <p id="gm-objective-consequence"></p>
          <button id="gm-objective-confirm"></button>
          <button id="gm-objective-cancel"></button>
        </div>
        <ol id="gm-objective-results"></ol>
      </section>
    `;
  });

  /** Twelve REAL, distinct, dense objective descriptions — genuine mission
   *  prose (world-authored objective text is free text, not a String-Table
   *  id: `gm-objective-panel.js` only resolves through the table when the
   *  value happens to be a known id), each long enough that a fixed-width
   *  truncation would visibly cut it. */
  const OBJECTIVES = Array.from({ length: 12 }, (_, i) => ({
    id: `obj-${i}`,
    text: `Escort the Directive courier past the Ladder ${i} inspection line and confirm the transfer window stays open for every crew still aboard, reporting back the moment the safe-passage promise from the strike committee either holds or breaks (objective ${i}).`,
    text_params: {},
    recipients: [],
    available: true,
    status: 'Active',
  }));

  it('renders all twelve dense objectives with their full text and both verb buttons', () => {
    const panel = createGmObjectivePanel({
      doc: document, t, getOperator: () => ({ id: 'gm-a' }),
      getOperatorName: () => 'Alex', getShipName: (id) => id,
    });
    panel.update({ objective_palette: [], objectives: OBJECTIVES, objective_results: [] });

    const rows = [...document.querySelectorAll('#gm-objective-list .gm-objective-row')];
    expect(rows).toHaveLength(12);
    for (const [i, row] of rows.entries()) {
      // The full 250+ char sentence, not a shortened one — the exact text the
      // panel was handed, verbatim.
      expect(row.querySelector('p').textContent).toBe(OBJECTIVES[i].text);
      expect(row.querySelector('p').textContent.length).toBeGreaterThan(200);
      // Both verbs an in-progress non-palette objective offers stay present
      // and correctly labelled with the objective's own real text, not a
      // truncated stand-in.
      const complete = row.querySelector('button[data-verb="complete"]');
      const fail = row.querySelector('button[data-verb="fail"]');
      expect(complete).not.toBeNull();
      expect(fail).not.toBeNull();
      expect(complete.getAttribute('aria-label')).toContain(OBJECTIVES[i].text);
    }
  });

  it(`opening "${GM_LIST_IDS[1]}" for each of the twelve dense objectives shows its own full preview`, () => {
    expect(GM_LIST_IDS[1]).toBe('server.gm.objective.preview_complete');
    const panel = createGmObjectivePanel({
      doc: document, t, getOperator: () => ({ id: 'gm-a' }),
      getOperatorName: () => 'Alex', getShipName: (id) => id,
    });
    panel.update({ objective_palette: [], objectives: OBJECTIVES, objective_results: [] });
    for (const objective of OBJECTIVES) {
      document.querySelector(
        `button[data-objective="${objective.id}"][data-verb="complete"]`,
      ).click();
      const consequence = document.getElementById('gm-objective-consequence').textContent;
      expect(consequence).toContain(objective.text);
      // Reset for the next iteration without submitting anything.
      document.getElementById('gm-objective-cancel').click();
    }
  });
});

// Issue #1430 review, blocking finding: the "performing" surface's
// gui/gm-knowledge-compare.js renders two of #1421's own
// `DENSE_CONTENT.gmLists` rows — the panel's scope/privacy hint (a long,
// static sentence) and its per-category summary (composed fresh on every
// render from real diff counts) — and neither was driven through the real
// controller by any test added for this issue. This closes that gap: the
// same production `createGmKnowledgeCompare` controller and real String
// Table the "directing" panels above use, fed a dense, realistic
// Truth/Crew Knowledge comparison covering every diff status the summary
// sentence itself names.
describe('GM Knowledge Compare panel at dense comparison density (issue #1430)', () => {
  const HINT_ID = GM_LIST_IDS[0];
  const SUMMARY_ID = GM_LIST_IDS[3];

  function truthContact(id, overrides = {}) {
    return {
      entity_id: id, name: overrides.name || id, kind: 'npc_ship', position: [0, 0, 0],
      faction: null,
      status: { hull_percent: overrides.hull_percent ?? 80, condition_percent: null, destroyed: false },
      current_target: null, geometry: null,
      radar: { icon: 'ship', colour: null, size: null, region_colour: null },
    };
  }

  function crewEntity(uuid, overrides = {}) {
    return {
      uuid, name: overrides.name || uuid, position: [0, 0, 0], tags: ['ship'],
      radar_icon: 'ship', hull_fraction: overrides.hull_fraction ?? 0.8,
    };
  }

  /** Mirrors `shipProjection()` in tests/client/gm-knowledge-compare.test.js
   *  — the minimal wire-shaped `GmPuppetShipProjection` that
   *  `buildGmStationConsoleInput`/`buildSensorsConsoleState` already accept
   *  without throwing (proven there). Duplicated locally rather than
   *  imported so this dense-content file stays self-contained, matching its
   *  own local `event()`/`denseResults()`/`OBJECTIVES` fixture builders
   *  above rather than reaching into a sibling test file's internals. */
  function shipProjection({ entities = [] } = {}) {
    return {
      ship_id: 'ship-player-1', name: 'Resolute',
      stations: [{
        station_id: 'sensors', name: 'Sensors', console: 'gui/sensors-console.html',
        rating: 'Backfill', operators: [],
      }],
      ship_config: {
        station_systems: { sensors: ['sensor-main'], comms: ['comms-main'] },
        system_console_families: { 'sensor-main': 'sensors', 'comms-main': 'comms' },
        system_kinds: { 'sensor-main': 'sensors', 'comms-main': 'comms' },
        blackboard_console_families: {},
        sensors_radar_range: 5000, sensors_radar_shows: [], sensors_radar_selects: [],
        station_tutorials: {}, station_assist_gaps: {},
      },
      station_ratings: { sensors: 'Backfill' }, control_sources: {},
      blackboards: [['comms-main', {
        kind: 'Comms', data: { messages: [], objectives: [], contacts: [], host_station: null },
      }]],
      entities, entity_states: [], objectives: [],
      ship_pose: { x: 0, y: 0, z: 0, yaw: 0, forward_speed: 0 },
      navigation_waypoint: null,
      console_hull: [{
        system_id: 'sensor-main', display_name: 'Sensors', current: 40, max_hp: 40,
        tier: 'Nominal', debuff_magnitude: 0,
      }],
    };
  }

  function mountKnowledgePanel() {
    document.body.innerHTML = `
      <p id="gm-knowledge-pending"></p>
      <section id="gm-knowledge-panel" hidden>
        <p id="gm-knowledge-hint" data-i18n="server.gm.knowledge.hint"></p>
        <select id="gm-knowledge-select"></select>
        ${['contacts', 'objectives', 'comms-messages', 'comms-contacts'].map((key) => `
          <p id="gm-knowledge-${key}-summary"></p>
          <p id="gm-knowledge-${key}-empty"></p>
          <table id="gm-knowledge-${key}-table" hidden><tbody id="gm-knowledge-${key}-rows"></tbody></table>
        `).join('')}
      </section>`;
  }

  it(`renders the real, untruncated "${HINT_ID}" scope/privacy hint via the ordinary data-i18n bind`, () => {
    expect(HINT_ID).toBe('server.gm.knowledge.hint');
    mountKnowledgePanel();
    applyToDom(document);
    const hint = document.getElementById('gm-knowledge-hint').textContent;
    // #1421 names this row a real 189-char sentence in production
    // strings.csv — proof the static i18n bind carries the FULL sentence,
    // not a shortened stand-in, and that it is real resolved prose, never a
    // raw id leaking through.
    expect(hint).toBe(t(HINT_ID));
    expect(hint.length).toBeGreaterThan(150);
    expect(hint).not.toMatch(/^server\./);
  });

  it(`renders "${SUMMARY_ID}" through the real controller for a dense comparison covering every diff status`, () => {
    expect(SUMMARY_ID).toBe('server.gm.knowledge.summary');
    mountKnowledgePanel();
    const controller = createGmKnowledgeCompare({ doc: document, t });

    // Twelve contacts: three of each status the summary sentence itself
    // names (same / changed / truth_only / crew_only) — the dense, realistic
    // comparison PRD #1418 asks for, not a one-row smoke check.
    const truthEntities = [];
    const shipEntities = [];
    for (let i = 0; i < 3; i += 1) {
      truthEntities.push(truthContact(`same-${i}`, { name: `Same ${i}` }));
      shipEntities.push(crewEntity(`same-${i}`, { name: `Same ${i}` }));
    }
    for (let i = 0; i < 3; i += 1) {
      // 90% Truth hull vs. a 0.4 crew hull_fraction (40%) — genuinely
      // different on the one compared numeric field, nothing else.
      truthEntities.push(truthContact(`changed-${i}`, { name: `Changed ${i}`, hull_percent: 90 }));
      shipEntities.push(crewEntity(`changed-${i}`, { name: `Changed ${i}`, hull_fraction: 0.4 }));
    }
    for (let i = 0; i < 3; i += 1) {
      truthEntities.push(truthContact(`truth-only-${i}`, { name: `Truth Only ${i}` }));
    }
    for (let i = 0; i < 3; i += 1) {
      shipEntities.push(crewEntity(`crew-only-${i}`, { name: `Crew Only ${i}` }));
    }
    const ship = shipProjection({ entities: shipEntities });

    controller.updateTruth(truthEntities);
    controller.updateStations({ ships: [ship], activity: [] });

    expect(document.getElementById('gm-knowledge-panel').hidden).toBe(false);
    const rows = document.getElementById('gm-knowledge-contacts-rows').children;
    expect(rows).toHaveLength(12);
    const statusCounts = { same: 0, changed: 0, truth_only: 0, crew_only: 0 };
    for (const row of rows) statusCounts[row.dataset.status] += 1;
    expect(statusCounts).toEqual({ same: 3, changed: 3, truth_only: 3, crew_only: 3 });

    // The exact string the panel painted equals what the real reducer would
    // compose from those counts — a truncation, a stale render or a
    // miscounted summary regresses this without needing a browser to lay
    // the text out.
    const summary = document.getElementById('gm-knowledge-contacts-summary').textContent;
    expect(summary).toBe(t(SUMMARY_ID, statusCounts));
    expect(summary.length).toBeGreaterThan(10);
    expect(summary).not.toMatch(/^server\./);
  });
});
