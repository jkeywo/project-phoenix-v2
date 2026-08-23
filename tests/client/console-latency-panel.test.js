// @vitest-environment jsdom
//
// Issue #1169 — the debug dock's console input-to-feedback latency table.
//
// The renderer lives in gui/console-latency-panel.js precisely so it can be
// driven here without a browser or a WASM bundle: it is a pure function of the
// ConsoleLatencyPayload JSON the bridge publishes. These tests pin the one
// property the table's honesty rests on — a segment that path cannot measure
// reads as absent, never as zero.

import { describe, it, expect } from 'vitest';
import {
  parseConsoleLatency,
  buildConsoleLatencyTable,
  renderConsoleLatencyPanel,
} from '../../gui/console-latency-panel.js';

function samplePayload() {
  return {
    schema_version: 1,
    actions: [
      {
        surface: 'PhoneConsole',
        action: 'fire_phaser',
        count: 12,
        expired: 4,
        input_to_send: { count: 12, p50_ms: 2.0, p75_ms: 3.0, max_ms: 9.0 },
        send_to_ack: { count: 12, p50_ms: 55.0, p75_ms: 72.0, max_ms: 210.0 },
        input_to_ack: { count: 12, p50_ms: 57.0, p75_ms: 75.0, max_ms: 214.0 },
      },
      {
        surface: 'SimHost',
        action: 'FirePhaser',
        count: 40,
        expired: 0,
        admit_to_broadcast: { count: 40, p50_ms: 0.4, p75_ms: 0.8, max_ms: 3.0 },
      },
    ],
  };
}

describe('parseConsoleLatency', () => {
  it('returns null before the first publish', () => {
    expect(parseConsoleLatency('')).toBeNull();
    expect(parseConsoleLatency(undefined)).toBeNull();
  });

  it('returns null on malformed input rather than throwing on the render path', () => {
    expect(parseConsoleLatency('{')).toBeNull();
    expect(parseConsoleLatency('{"schema_version":1}')).toBeNull();
  });

  it('parses a well-formed payload', () => {
    const payload = parseConsoleLatency(JSON.stringify(samplePayload()));
    expect(payload.actions).toHaveLength(2);
  });
});

describe('buildConsoleLatencyTable', () => {
  it('draws one row per (surface, action)', () => {
    const root = buildConsoleLatencyTable(samplePayload(), { doc: document });
    const rows = root.querySelectorAll('.cl-row');
    expect(rows).toHaveLength(2);
    expect(rows[0].getAttribute('data-surface')).toBe('PhoneConsole');
    expect(rows[0].getAttribute('data-action')).toBe('fire_phaser');
    expect(rows[1].getAttribute('data-surface')).toBe('SimHost');
  });

  it('shows p50/p75/max together for a measured segment', () => {
    const root = buildConsoleLatencyTable(samplePayload(), { doc: document });
    const cell = root.querySelector('.cl-row[data-action="fire_phaser"] [data-segment="send_to_ack"]');
    expect(cell.textContent).toBe('55/72/210');
  });

  /// The property the whole table rests on: the simulation cannot see a
  /// player's input event, so its row must not claim a number for it.
  it('renders an unmeasurable segment as absent, never as zero', () => {
    const root = buildConsoleLatencyTable(samplePayload(), { doc: document });
    const row = root.querySelector('.cl-row[data-surface="SimHost"]');
    expect(row.querySelector('[data-segment="input_to_send"]').textContent).toBe('—');
    expect(row.querySelector('[data-segment="send_to_ack"]').textContent).toBe('—');
    expect(row.querySelector('[data-segment="admit_to_broadcast"]').textContent).toBe('0.4/0.8/3.0');
  });

  /// The in-process host path runs well under a millisecond; rounding it to
  /// whole milliseconds would print "0/0/0" and hide the very contrast with the
  /// phone path the table exists to show.
  it('keeps sub-millisecond figures readable', () => {
    const root = buildConsoleLatencyTable(samplePayload(), { doc: document });
    const cell = root.querySelector('.cl-row[data-surface="SimHost"] [data-segment="admit_to_broadcast"]');
    expect(cell.textContent).toContain('0.4');
  });
});

describe('renderConsoleLatencyPanel', () => {
  it('shows the empty state before anything is measured', () => {
    const host = document.createElement('div');
    renderConsoleLatencyPanel(host, '');
    expect(host.querySelector('.cl-empty')).toBeTruthy();
    expect(host.querySelector('.cl-table')).toBeNull();
  });

  it('shows the empty state for a payload with no measurements', () => {
    const host = document.createElement('div');
    renderConsoleLatencyPanel(host, JSON.stringify({ schema_version: 1, actions: [] }));
    expect(host.querySelector('.cl-empty')).toBeTruthy();
  });

  it('replaces previous content on each render', () => {
    const host = document.createElement('div');
    renderConsoleLatencyPanel(host, JSON.stringify(samplePayload()));
    renderConsoleLatencyPanel(host, JSON.stringify(samplePayload()));
    expect(host.querySelectorAll('.cl-table')).toHaveLength(1);
  });

  it('tolerates a missing container', () => {
    expect(() => renderConsoleLatencyPanel(null, '{}')).not.toThrow();
  });
});

// ── The outage counter (issue #1169 review, finding C1) ─────────────────────
//
// An unanswered action contributes no duration, so the distributions alone
// cannot tell a dead link from a quiet one. The count has to be visible beside
// them or the table is quietly reassuring at exactly the wrong moment.

describe('buildConsoleLatencyTable — outage counter', () => {
  it('shows the unanswered count for each row', () => {
    const root = buildConsoleLatencyTable(samplePayload(), { doc: document });
    const phone = root.querySelector('.cl-row[data-surface="PhoneConsole"] .cl-expired');
    expect(phone.textContent).toBe('4');
    expect(phone.classList.contains('cl-expired-warn')).toBe(true);
  });

  it('does not flag a row with no outages', () => {
    const root = buildConsoleLatencyTable(samplePayload(), { doc: document });
    const host = root.querySelector('.cl-row[data-surface="SimHost"] .cl-expired');
    expect(host.textContent).toBe('0');
    expect(host.classList.contains('cl-expired-warn')).toBe(false);
  });

  it('reads a payload from a host that predates the counter as zero', () => {
    const payload = samplePayload();
    delete payload.actions[0].expired;
    const root = buildConsoleLatencyTable(payload, { doc: document });
    const phone = root.querySelector('.cl-row[data-surface="PhoneConsole"] .cl-expired');
    expect(phone.textContent).toBe('0');
  });
});

// The host page mounts no console surface that receives state, so it has no
// acknowledgement to measure and produces no series (issue #1169 review, B2).
// A payload can therefore only carry the two surfaces the table names.
describe('buildConsoleLatencyTable — surfaces', () => {
  it('labels an unknown surface with its own name rather than inventing one', () => {
    const payload = { schema_version: 1, actions: [{ surface: 'BrowserHost', action: 'x', count: 1, expired: 0 }] };
    const root = buildConsoleLatencyTable(payload, { doc: document });
    expect(root.querySelector('.cl-surface').textContent).toBe('BrowserHost');
  });
});
