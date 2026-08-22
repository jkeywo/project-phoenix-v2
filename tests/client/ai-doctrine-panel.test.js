// @vitest-environment jsdom
//
// Issue #1149 — the debug dock's AI doctrine-pool panel.
//
// The renderer lives in gui/ai-doctrine-panel.js precisely so it can be driven
// here without a browser or a WASM bundle: it is a pure function of the
// AiStatePayload JSON the bridge publishes. These tests pin that it parses the
// payload defensively and draws, per ship, the scored-objective pool with each
// candidate's score, directive and resolved target — and marks the chosen one.

import { describe, it, expect } from 'vitest';
import {
  parseAiDoctrine,
  buildAiDoctrinePanel,
  renderAiDoctrinePanel,
} from '../../gui/ai-doctrine-panel.js';

/** A payload matching `codec::encode_ai_doctrine`'s wire shape. */
function samplePayload() {
  return {
    schema_version: 1,
    tick: 42,
    ships: [
      {
        ship: 'Ashrender',
        uuid: 'uuid-a',
        chosen: { id: 'kill', directive: 'Destroy(Harrow)', target: 'Harrow', score: 38.0 },
        candidates: [
          {
            id: 'kill',
            score: 38.0,
            source: 'Doctrine',
            relevance: ['Weapons'],
            directive: 'Destroy(Harrow)',
            target: 'Harrow',
            mandatory: true,
            status: 'Active',
          },
          {
            id: 'patrol',
            score: 12.0,
            source: 'Doctrine',
            relevance: ['Helm'],
            directive: 'Patrol(picket loop)',
            target: 'picket',
            mandatory: false,
            status: 'Active',
          },
        ],
      },
      {
        ship: 'Idle',
        candidates: [],
      },
    ],
  };
}

/** A payload with the per-host policy surface (issue #1152). */
function sampleHostPayload() {
  return {
    schema_version: 1,
    tick: 5,
    ships: [],
    hosts: [
      {
        ship: 'Harrow',
        uuid: 'uuid-harrow',
        host: 'Helm boost',
        state: 'surge',
        entered_at_secs: 2.0,
        memory: [
          { key: 'engagements', value: 1 },
          { key: 'peak_hazard', value: 0.8 },
        ],
        last_transition: {
          from: 'cruise',
          to: 'surge',
          guard: 'fact(hazard_urgency) > param(surge)',
          at_secs: 2.0,
        },
        blocked_transition: {
          from: 'surge',
          to: 'cruise',
          guard: 'state_time >= param(dwell)',
        },
      },
    ],
  };
}

describe('parseAiDoctrine', () => {
  it('parses a well-formed payload', () => {
    const payload = parseAiDoctrine(JSON.stringify(samplePayload()));
    expect(payload).not.toBeNull();
    expect(payload.ships).toHaveLength(2);
  });

  it('returns null for an empty string (before the first publish)', () => {
    expect(parseAiDoctrine('')).toBeNull();
  });

  it('returns null for malformed JSON rather than throwing', () => {
    expect(parseAiDoctrine('{not json')).toBeNull();
  });

  it('returns null when ships is missing', () => {
    expect(parseAiDoctrine('{"schema_version":1}')).toBeNull();
  });
});

describe('buildAiDoctrinePanel', () => {
  it('draws one section per ship', () => {
    const panel = buildAiDoctrinePanel(samplePayload(), { doc: document });
    const ships = panel.querySelectorAll('.ad-ship');
    expect(ships).toHaveLength(2);
    expect(ships[0].getAttribute('data-ship')).toBe('Ashrender');
    expect(ships[1].getAttribute('data-ship')).toBe('Idle');
  });

  it('lists every candidate with its score, directive and resolved target', () => {
    const panel = buildAiDoctrinePanel(samplePayload(), { doc: document });
    const kill = panel.querySelector('.ad-ship[data-ship="Ashrender"] .ad-candidate[data-objective="kill"]');
    expect(kill).not.toBeNull();
    expect(kill.querySelector('.ad-c-score').textContent).toBe('38.0');
    expect(kill.querySelector('.ad-c-directive').textContent).toBe('Destroy(Harrow)');
    expect(kill.querySelector('.ad-c-target').textContent).toBe('Harrow');
    expect(kill.querySelector('.ad-c-status').textContent).toBe('Active');
  });

  it('marks the chosen directive row', () => {
    const panel = buildAiDoctrinePanel(samplePayload(), { doc: document });
    const chosen = panel.querySelectorAll('.ad-ship[data-ship="Ashrender"] .ad-candidate.chosen');
    expect(chosen).toHaveLength(1);
    expect(chosen[0].getAttribute('data-objective')).toBe('kill');
  });

  it('flags a mandatory candidate', () => {
    const panel = buildAiDoctrinePanel(samplePayload(), { doc: document });
    const kill = panel.querySelector('.ad-candidate[data-objective="kill"]');
    const patrol = panel.querySelector('.ad-candidate[data-objective="patrol"]');
    expect(kill.classList.contains('mandatory')).toBe(true);
    expect(patrol.classList.contains('mandatory')).toBe(false);
  });

  it('renders a ship with no chosen directive without a chosen row', () => {
    const panel = buildAiDoctrinePanel(samplePayload(), { doc: document });
    const idle = panel.querySelector('.ad-ship[data-ship="Idle"]');
    expect(idle.querySelector('.ad-chosen-none')).not.toBeNull();
    expect(idle.querySelectorAll('.ad-candidate')).toHaveLength(0);
  });
});

describe('buildAiDoctrinePanel — per-host policy view (issue #1152)', () => {
  it('draws one section per stateful host with its current state', () => {
    const panel = buildAiDoctrinePanel(sampleHostPayload(), { doc: document });
    const hosts = panel.querySelectorAll('.ah-host');
    expect(hosts).toHaveLength(1);
    expect(hosts[0].getAttribute('data-ship')).toBe('Harrow');
    expect(hosts[0].getAttribute('data-host')).toBe('Helm boost');
    expect(hosts[0].querySelector('.ah-state').getAttribute('data-state')).toBe('surge');
  });

  it('lists the private memory readings', () => {
    const panel = buildAiDoctrinePanel(sampleHostPayload(), { doc: document });
    const mem = panel.querySelector('.ah-host .ah-mem[data-key="engagements"]');
    expect(mem).not.toBeNull();
    expect(mem.querySelector('.ah-m-value').textContent).toBe('1');
    expect(panel.querySelector('.ah-mem[data-key="peak_hazard"]')).not.toBeNull();
  });

  it('shows the last committed transition with its guard', () => {
    const panel = buildAiDoctrinePanel(sampleHostPayload(), { doc: document });
    const last = panel.querySelector('.ah-host .ah-transition.ah-last');
    expect(last).not.toBeNull();
    expect(last.querySelector('.ah-t-edge').textContent).toBe('cruise → surge');
    expect(last.querySelector('.ah-t-guard').textContent).toBe(
      'fact(hazard_urgency) > param(surge)',
    );
  });

  it('shows the blocked transition with the guard holding it', () => {
    const panel = buildAiDoctrinePanel(sampleHostPayload(), { doc: document });
    const blocked = panel.querySelector('.ah-host .ah-transition.ah-blocked');
    expect(blocked).not.toBeNull();
    expect(blocked.querySelector('.ah-t-edge').textContent).toBe('surge → cruise');
    expect(blocked.querySelector('.ah-t-guard').textContent).toBe('state_time >= param(dwell)');
  });

  it('omits transition rows a machine has not recorded', () => {
    const payload = sampleHostPayload();
    delete payload.hosts[0].last_transition;
    delete payload.hosts[0].blocked_transition;
    const panel = buildAiDoctrinePanel(payload, { doc: document });
    expect(panel.querySelectorAll('.ah-transition')).toHaveLength(0);
    // The host is still drawn — its state and memory remain.
    expect(panel.querySelector('.ah-host')).not.toBeNull();
  });
});

describe('renderAiDoctrinePanel', () => {
  it('renders the panel into a container from raw JSON', () => {
    const container = document.createElement('div');
    renderAiDoctrinePanel(container, JSON.stringify(samplePayload()));
    expect(container.querySelector('.ad-panel')).not.toBeNull();
    expect(container.querySelectorAll('.ad-ship')).toHaveLength(2);
  });

  it('shows the empty placeholder before any data arrives', () => {
    const container = document.createElement('div');
    renderAiDoctrinePanel(container, '');
    expect(container.querySelector('.ad-empty')).not.toBeNull();
    expect(container.querySelector('.ad-panel')).toBeNull();
  });

  it('shows the empty placeholder when no AI ships are present', () => {
    const container = document.createElement('div');
    renderAiDoctrinePanel(container, JSON.stringify({ schema_version: 1, tick: 0, ships: [] }));
    expect(container.querySelector('.ad-empty')).not.toBeNull();
    expect(container.querySelector('.ad-panel')).toBeNull();
  });

  it('renders the panel when only per-host policy machines are present', () => {
    const container = document.createElement('div');
    renderAiDoctrinePanel(container, JSON.stringify(sampleHostPayload()));
    expect(container.querySelector('.ad-panel')).not.toBeNull();
    expect(container.querySelector('.ad-empty')).toBeNull();
    expect(container.querySelectorAll('.ah-host')).toHaveLength(1);
  });

  it('clears prior content on each render', () => {
    const container = document.createElement('div');
    renderAiDoctrinePanel(container, JSON.stringify(samplePayload()));
    renderAiDoctrinePanel(container, '');
    expect(container.querySelectorAll('.ad-panel')).toHaveLength(0);
    expect(container.querySelectorAll('.ad-empty')).toHaveLength(1);
  });
});
