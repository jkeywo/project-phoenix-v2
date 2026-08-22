// @vitest-environment jsdom
//
// Issue #1148 — the debug dock's scenario-state panel.
//
// The renderer lives in gui/scenario-state-panel.js precisely so it can be
// driven here without a browser or a WASM bundle: it is a pure function of the
// ScenarioStatePayload JSON the bridge publishes. These tests pin that it parses
// the payload defensively and lays out every surface — flags, objectives,
// triggers with eligibility, queues, commitments and dossier — with the
// data-attributes an author reads the state off.

import { describe, it, expect } from 'vitest';
import {
  parseScenarioState,
  buildScenarioStatePanel,
  renderScenarioStatePanel,
} from '../../gui/scenario-state-panel.js';

/** A payload matching `codec::encode_scenario_state`'s wire shape. */
function samplePayload() {
  return {
    schema_version: 1,
    flags: [
      { name: 'alarm', value: 1 },
      { name: 'wave', value: 3 },
    ],
    objectives: [
      {
        id: 'kill',
        status: 'Active',
        mandatory: true,
        base_priority: 7,
        directive: { kind: 'Destroy', target: 'raider' },
      },
      {
        id: 'scan',
        status: 'Completed',
        mandatory: false,
        base_priority: 2,
        directive: { kind: 'None' },
      },
    ],
    triggers: [
      {
        id: 'beat',
        condition: 'on_timer(after_secs=30)',
        when: 'flag(ready)',
        repeat: false,
        fired: false,
        pending: true,
        when_holds: false,
      },
      {
        id: 'loaded',
        condition: 'on_world_loaded',
        repeat: false,
        fired: true,
        pending: false,
        when_holds: true,
        // Fire history (#1151): one recorded fire with its predicate values.
        fire_history: [
          {
            fired_secs: 12.5,
            predicate_values: [
              { atom: 'flag(ready)', value: 'true' },
              { atom: 'counter(kills)', value: '5' },
            ],
          },
        ],
      },
    ],
    delayed_actions: [{ action: 'set_world_flag(reinforce)', fire_at_secs: 45 }],
    deadlines: [
      { id: 'window', label: 'world.deadline.window', visible: true, due_tick: 36000, state: 'pending' },
    ],
    commitments: [
      { id: 'passage', made_to: 'strike_committee', terms: 'terms.passage', state: 'open', made_at_tick: 10 },
    ],
    dossier: [
      { subject_uuid: 'uuid-1', text: 'evidence.forged_manifest', provenance: 'records', gathered_at_tick: 40 },
    ],
  };
}

describe('parseScenarioState', () => {
  it('parses a well-formed payload', () => {
    const payload = parseScenarioState(JSON.stringify(samplePayload()));
    expect(payload).not.toBeNull();
    expect(payload.objectives).toHaveLength(2);
  });

  it('returns null for an empty string (before the first publish)', () => {
    expect(parseScenarioState('')).toBeNull();
  });

  it('returns null for malformed JSON rather than throwing', () => {
    expect(parseScenarioState('{not json')).toBeNull();
  });

  it('returns null when the surface arrays are missing', () => {
    expect(parseScenarioState('{"schema_version":1}')).toBeNull();
  });
});

describe('buildScenarioStatePanel', () => {
  it('lays out every surface as its own section', () => {
    const panel = buildScenarioStatePanel(samplePayload(), { doc: document });
    for (const id of [
      'flags',
      'objectives',
      'triggers',
      'delayed',
      'deadlines',
      'commitments',
      'dossier',
    ]) {
      expect(
        panel.querySelector(`.ss-section[data-section="${id}"]`),
        `missing section ${id}`,
      ).not.toBeNull();
    }
  });

  it('renders each flag with its value', () => {
    const panel = buildScenarioStatePanel(samplePayload(), { doc: document });
    const alarm = panel.querySelector('.ss-flag[data-flag="alarm"]');
    expect(alarm).not.toBeNull();
    expect(alarm.querySelector('.ss-flag-value').textContent).toBe('1');
  });

  it('renders each objective with id, status, priority and directive', () => {
    const panel = buildScenarioStatePanel(samplePayload(), { doc: document });
    const kill = panel.querySelector('.ss-objective[data-id="kill"]');
    expect(kill.getAttribute('data-status')).toBe('Active');
    expect(kill.getAttribute('data-mandatory')).toBe('true');
    expect(kill.querySelector('.ss-objective-priority').textContent).toBe('7');
    expect(kill.querySelector('.ss-objective-directive').getAttribute('data-directive')).toBe(
      'Destroy',
    );
  });

  it('marks a pending trigger waiting on its gate distinctly from a fired one', () => {
    const panel = buildScenarioStatePanel(samplePayload(), { doc: document });
    const beat = panel.querySelector('.ss-trigger[data-id="beat"]');
    // Armed but its `when` gate is not holding: the "waiting" evidence.
    expect(beat.getAttribute('data-pending')).toBe('true');
    expect(beat.getAttribute('data-fired')).toBe('false');
    expect(beat.getAttribute('data-when-holds')).toBe('false');
    expect(beat.querySelector('.ss-trigger-status').getAttribute('data-state')).toBe('waiting');
    expect(beat.querySelector('.ss-trigger-when').getAttribute('data-holds')).toBe('false');

    const loaded = panel.querySelector('.ss-trigger[data-id="loaded"]');
    expect(loaded.getAttribute('data-fired')).toBe('true');
    expect(loaded.querySelector('.ss-trigger-status').getAttribute('data-state')).toBe('fired');
  });

  it('renders a fired trigger fire history with its predicate values (#1151)', () => {
    const panel = buildScenarioStatePanel(samplePayload(), { doc: document });
    const loaded = panel.querySelector('.ss-trigger[data-id="loaded"]');
    const fires = loaded.querySelector('.ss-trigger-fires');
    expect(fires, 'the fired trigger shows its fire history').not.toBeNull();
    expect(fires.getAttribute('data-count')).toBe('1');
    const fire = fires.querySelector('.ss-fire');
    expect(fire.getAttribute('data-fired-secs')).toBe('12.5');
    // Each predicate atom reads back in the condition/when vocabulary with its
    // observed value.
    const ready = fire.querySelector('.ss-fire-value[data-atom="flag(ready)"]');
    expect(ready).not.toBeNull();
    expect(ready.textContent).toBe('flag(ready) = true');
    expect(
      fire.querySelector('.ss-fire-value[data-atom="counter(kills)"]').textContent,
    ).toBe('counter(kills) = 5');
  });

  it('omits the fire-history block for a trigger that has not fired (#1151)', () => {
    const panel = buildScenarioStatePanel(samplePayload(), { doc: document });
    // The 'beat' trigger has no fire_history key at all — a pre-fire / pre-#1151
    // payload — and must render without throwing and without a fires block.
    const beat = panel.querySelector('.ss-trigger[data-id="beat"]');
    expect(beat.querySelector('.ss-trigger-fires')).toBeNull();
  });

  it('renders the deadline and commitment states', () => {
    const panel = buildScenarioStatePanel(samplePayload(), { doc: document });
    expect(
      panel.querySelector('.ss-deadline[data-id="window"]').getAttribute('data-state'),
    ).toBe('pending');
    expect(
      panel.querySelector('.ss-commitment[data-id="passage"]').getAttribute('data-state'),
    ).toBe('open');
  });

  it('renders a dossier finding with its provenance', () => {
    const panel = buildScenarioStatePanel(samplePayload(), { doc: document });
    const entry = panel.querySelector('.ss-dossier-entry[data-provenance="records"]');
    expect(entry).not.toBeNull();
    expect(entry.querySelector('.ss-dossier-text').textContent).toBe('evidence.forged_manifest');
  });

  it('shows a (none) row for an empty surface rather than hiding the section', () => {
    const payload = samplePayload();
    payload.commitments = [];
    const panel = buildScenarioStatePanel(payload, { doc: document });
    const sec = panel.querySelector('.ss-section[data-section="commitments"]');
    expect(sec.querySelector('.ss-none')).not.toBeNull();
    expect(sec.querySelector('.ss-commitment')).toBeNull();
  });
});

describe('renderScenarioStatePanel', () => {
  it('renders the panel into a container from raw JSON', () => {
    const container = document.createElement('div');
    renderScenarioStatePanel(container, JSON.stringify(samplePayload()));
    expect(container.querySelector('.ss-panel')).not.toBeNull();
    expect(container.querySelectorAll('.ss-objective')).toHaveLength(2);
  });

  it('shows the empty placeholder before any data arrives', () => {
    const container = document.createElement('div');
    renderScenarioStatePanel(container, '');
    expect(container.querySelector('.ss-empty')).not.toBeNull();
    expect(container.querySelector('.ss-panel')).toBeNull();
  });

  it('shows the empty placeholder when every surface is empty', () => {
    const container = document.createElement('div');
    const empty = {
      schema_version: 1,
      flags: [],
      objectives: [],
      triggers: [],
      delayed_actions: [],
      deadlines: [],
      commitments: [],
      dossier: [],
    };
    renderScenarioStatePanel(container, JSON.stringify(empty));
    expect(container.querySelector('.ss-empty')).not.toBeNull();
    expect(container.querySelector('.ss-panel')).toBeNull();
  });

  it('clears prior content on each render', () => {
    const container = document.createElement('div');
    renderScenarioStatePanel(container, JSON.stringify(samplePayload()));
    renderScenarioStatePanel(container, '');
    expect(container.querySelectorAll('.ss-panel')).toHaveLength(0);
    expect(container.querySelectorAll('.ss-empty')).toHaveLength(1);
  });
});
