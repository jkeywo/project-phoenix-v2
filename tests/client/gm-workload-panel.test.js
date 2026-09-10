// @vitest-environment jsdom
/**
 * tests/client/gm-workload-panel.test.js — the GM Station-workload advisory
 * (issue #1438, PRD #1419 M4 stories 11/12/14, presentation contract PRD
 * #1418).
 *
 * Drives the shipped `createGmWorkloadPanel` against the exact markup
 * `server.html` carries, plus the shipped `createHostChannel` for the raw-DTO
 * passthrough. jsdom lays nothing out, so the measured half of the 200% claim
 * is the CSS contract read at the bottom of this file plus
 * `tests/smoke/gm-layout.spec.js`, which drives the real desk at 1280×720 with
 * `--a11y-text-scale: 2`.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import {
  createGmWorkloadPanel,
  parseGmWorkloadProjection,
  workloadCountsPeople,
  GM_WORKLOAD_LEVELS,
} from '../../gui/gm-workload-panel.js';
import { createHostChannel } from '../../gui/host-channel.js';

/** The exact ids server.html carries for the panel. */
const MARKUP = `
  <section id="gm-workload-panel">
    <h2 id="gm-workload-heading"></h2>
    <p id="gm-workload-status"></p>
    <ul id="gm-workload-list"></ul>
    <p id="gm-workload-empty"></p>
  </section>`;

const t = (id, params = {}) => Object.entries(params)
  .reduce((text, [key, value]) => text.replaceAll(`{${key}}`, String(value)), id);

function station(extra = {}) {
  return {
    ship: { entity_id: 'ship-a', name: 'Alpha' },
    station_id: 'comms',
    station_name: 'Comms',
    level: 'engaged',
    count: 1,
    sustained_secs: 0,
    overload_count: 3,
    overload_secs: 30,
    demands: [{
      key: 'comms:m1',
      source: 'pending_comms',
      reason: { id: 'server.gm.workload.reason.pending_comms', params: { sender: 'Axiom' } },
    }],
    ...extra,
  };
}

const payload = (...rows) => ({ stations: rows });

function mount({ doc = document, copy = null } = {}) {
  doc.body.innerHTML = MARKUP;
  // With `copy`, ids resolve to that authored sentence and parameters are
  // interpolated into it — the String Table's own behaviour. Without it, the
  // default stub returns the id, which is what most cases here assert on.
  const resolve = copy
    ? (id, params = {}) => Object.entries(params)
      .reduce((text, [key, value]) => text.replaceAll(`{${key}}`, String(value)),
        Object.hasOwn(copy, id) ? copy[id] : id)
    : t;
  return createGmWorkloadPanel({
    doc,
    t: resolve,
    has: (id) => typeof id === 'string' && id.startsWith('server.'),
  });
}

const rowKeys = (doc = document) => [...doc.querySelectorAll('#gm-workload-list li[data-station-key]')]
  .map((row) => row.dataset.stationKey);
const rowFor = (key, doc = document) => doc
  .querySelector(`#gm-workload-list li[data-station-key="${key}"]`);

beforeEach(() => { vi.useFakeTimers(); vi.setSystemTime(new Date('2026-09-10T10:00:00Z')); });
afterEach(() => { vi.useRealTimers(); });

describe('GM workload projection parsing', () => {
  it('drops malformed rows and refuses a payload that is not a summary', () => {
    expect(parseGmWorkloadProjection('not json')).toBe(null);
    expect(parseGmWorkloadProjection({ rows: [] })).toBe(null);
    const parsed = parseGmWorkloadProjection(JSON.stringify(payload(
      station(),
      station(),                                  // duplicate key
      station({ level: 'exhausted' }),            // an invented level
      station({ station_id: 'helm', count: -1 }), // an impossible count
      station({ station_id: 'science', ship: {} }),
      station({ station_id: 'tactical', level: 'backfill', count: 0, demands: [
        { key: '', reason: { id: 'x' } },
        { key: 'ok', reason: {} },
      ] }),
    )));
    expect(parsed.stations.map((row) => row.station_id)).toEqual(['comms', 'tactical']);
    // A demand with no key or no reason id is not evidence of anything.
    expect(parsed.stations[1].demands).toEqual([]);
  });

  it('knows which levels are a claim about a person', () => {
    expect(GM_WORKLOAD_LEVELS).toContain('backfill');
    expect(workloadCountsPeople('overloaded')).toBe(true);
    expect(workloadCountsPeople('backfill')).toBe(false);
    expect(workloadCountsPeople('offline')).toBe(false);
  });

  it('reaches the panel as a raw DTO, with its reason ids unresolved', () => {
    const seen = [];
    const dispatch = createHostChannel({
      handlers: { gm_workload: (p) => seen.push(p) },
      strings: {
        t, has: () => true,
        localiseTree: () => {},
      },
    });
    dispatch('gm_workload', payload(station()));
    expect(seen).toHaveLength(1);
    // Not substituted on the way in: the panel has to interpolate {sender}
    // into the sentence itself, which it cannot do once the id is gone.
    expect(seen[0].stations[0].demands[0].reason.id)
      .toBe('server.gm.workload.reason.pending_comms');
    expect(seen[0].stations[0].station_id).toBe('comms');
  });
});

describe('GM workload panel', () => {
  it('says the level in words and names the demands behind the count', () => {
    const panel = mount();
    panel.update(payload(station()));
    const row = rowFor('ship-a/comms');
    expect(row.querySelector('.gm-workload-state').textContent)
      .toBe('server.gm.workload.state.engaged');
    expect(row.querySelector('.gm-workload-count').textContent)
      .toBe('server.gm.workload.count');
    // The evidence is the demands themselves, not a score.
    row.querySelector('summary').click();
    const demands = [...row.querySelectorAll('.gm-workload-demands li')];
    expect(demands).toHaveLength(1);
    expect(demands[0].textContent).toBe('server.gm.workload.reason.pending_comms');
    expect(demands[0].dataset.source).toBe('pending_comms');
    expect(row.textContent).not.toMatch(/%/);
    panel.dispose();
  });

  it('counts crewed Stations in its own status sentence, not the per-row one', () => {
    // The two shipped sentences, as `assets/strings/strings.csv` authors them.
    const panel = mount({
      copy: {
        'server.gm.workload.summary': 'Crewed Stations: {count}',
        'server.gm.workload.count': '{count} waiting',
      },
    });
    panel.update(payload(
      station(),
      station({ station_id: 'helm', station_name: 'Helm', level: 'underused', count: 0, demands: [] }),
      station({ station_id: 'tactical', station_name: 'Tactical', level: 'backfill', count: 0, demands: [] }),
      station({ station_id: 'science', station_name: 'Science', level: 'offline', count: 0, demands: [] }),
    ));
    // Two of the four Stations have a person at them. The sentence says so in
    // its OWN copy: the per-row `…count` id reads "N waiting", which would have
    // claimed the panel was waiting on two things it is not.
    expect(document.getElementById('gm-workload-status').textContent).toBe('Crewed Stations: 2');
    panel.update(payload(station()));
    expect(document.getElementById('gm-workload-status').textContent).toBe('Crewed Stations: 1');
    panel.dispose();
  });

  it('resolves a demand parameter that is itself a String Table id', () => {
    // The two shipped sentences, as `assets/strings/strings.csv` authors them.
    // The tier is a PARAMETER of the reason and is itself an id, so proving the
    // fix means reading the finished sentence, not the outer id.
    const panel = mount({
      copy: {
        'server.gm.workload.reason.repair_dispatch':
          '{station} is asking for a repair team ({tier}).',
        'server.gm.workload.tier.disabled': 'badly damaged and offline',
      },
    });
    panel.update(payload(station({
      station_id: 'engineering',
      station_name: 'Engineering',
      demands: [{
        key: 'repair:ship-a/weapons',
        source: 'repair_dispatch',
        reason: {
          id: 'server.gm.workload.reason.repair_dispatch',
          params: { station: 'Tactical', tier: 'server.gm.workload.tier.disabled' },
        },
      }],
    })));
    const row = rowFor('ship-a/engineering');
    row.querySelector('summary').click();
    const demand = row.querySelector('.gm-workload-demands li');
    // The tier travels as an id and is resolved through the table, so what a
    // facilitator reads is copy somebody wrote — never raw Rust `Debug` output
    // and never a bare id left sitting inside the sentence.
    expect(demand.textContent)
      .toBe('Tactical is asking for a repair team (badly damaged and offline).');
    expect(demand.textContent).not.toMatch(/Disabled/);
    expect(demand.textContent).not.toMatch(/server\./);
    panel.dispose();
  });

  it('shows Backfill instead of a count, and never any human evidence', () => {
    const panel = mount();
    panel.update(payload(
      station({ level: 'backfill', count: 0, demands: [] }),
      station({ station_id: 'engineering', station_name: 'Engineering', level: 'offline', count: 0, demands: [] }),
    ));
    const backfilled = rowFor('ship-a/comms');
    expect(backfilled.querySelector('.gm-workload-state').textContent)
      .toBe('server.gm.workload.state.backfill');
    expect(backfilled.querySelector('.gm-workload-count')).toBe(null);
    backfilled.querySelector('summary').click();
    expect(backfilled.querySelector('.gm-workload-note').textContent)
      .toBe('server.gm.workload.backfill_note');
    expect(backfilled.querySelector('.gm-workload-demands')).toBe(null);

    const offline = rowFor('ship-a/engineering');
    expect(offline.querySelector('.gm-workload-state').textContent)
      .toBe('server.gm.workload.state.offline');
    offline.querySelector('summary').click();
    expect(offline.querySelector('.gm-workload-note').textContent)
      .toBe('server.gm.workload.offline_note');
    panel.dispose();
  });

  it('explains why a Station at the threshold is still only Engaged', () => {
    const panel = mount();
    panel.update(payload(station({ count: 3, sustained_secs: 12, level: 'engaged' })));
    const row = rowFor('ship-a/comms');
    row.querySelector('summary').click();
    expect(row.querySelector('.gm-workload-building').textContent)
      .toBe('server.gm.workload.building');
    // Once it holds, the sentence goes: the level is the answer now.
    panel.update(payload(station({ count: 3, sustained_secs: 30, level: 'overloaded' })));
    const held = rowFor('ship-a/comms');
    expect(held.querySelector('.gm-workload-state').textContent)
      .toBe('server.gm.workload.state.overloaded');
    expect(held.querySelector('.gm-workload-building')).toBe(null);
    panel.dispose();
  });

  it('keeps a Station open, in place and under the keyboard while its count changes', () => {
    const panel = mount();
    panel.update(payload(
      station(),
      station({ station_id: 'helm', station_name: 'Helm', level: 'underused', count: 0, demands: [] }),
    ));
    expect(rowKeys()).toEqual(['ship-a/comms', 'ship-a/helm']);
    const summary = rowFor('ship-a/comms').querySelector('summary');
    summary.click();
    summary.focus();
    expect(panel.isExpanded('ship-a/comms')).toBe(true);
    expect(document.activeElement).toBe(summary);

    // A busier Comms arrives, and the Helm row becomes the busy one. Nothing
    // reorders, the expansion survives, and the keyboard is still in the row
    // it was reading.
    panel.update(payload(
      station({ count: 3, level: 'overloaded', sustained_secs: 30, demands: [
        { key: 'comms:m1', source: 'pending_comms', reason: { id: 'server.gm.workload.reason.pending_comms', params: { sender: 'Axiom' } } },
        { key: 'comms:m2', source: 'pending_comms', reason: { id: 'server.gm.workload.reason.pending_comms', params: { sender: 'Cordon' } } },
        { key: 'nav:ship-a#4', source: 'navigation_clearance', reason: { id: 'server.gm.workload.reason.navigation_clearance', params: { x: '10', z: '20' } } },
      ] }),
      station({ station_id: 'helm', station_name: 'Helm', level: 'engaged', count: 2, demands: [] }),
    ));
    expect(rowKeys()).toEqual(['ship-a/comms', 'ship-a/helm']);
    expect(panel.isExpanded('ship-a/comms')).toBe(true);
    const reopened = rowFor('ship-a/comms');
    expect(reopened.querySelector('details').open).toBe(true);
    expect(reopened.querySelectorAll('.gm-workload-demands li')).toHaveLength(3);
    expect(document.activeElement).toBe(reopened.querySelector('summary'));
    panel.dispose();
  });

  it('draws no row for a Station the projection omits, and counts only what it drew', () => {
    // The stock cruiser shape (issue #1438). While nobody is at Comms, the
    // `comms` System is presented at the Captain's seat: the projection omits
    // the Comms Station entirely and the Captain's row carries its count. An
    // omitted Station is simply NOT A ROW — not an Offline one, not a Backfill
    // one, and not a zero — so it can neither claim a seat is broken nor move
    // the status count.
    const copy = {
      'server.gm.workload.summary': 'Crewed Stations: {count}',
      'server.gm.workload.count': '{count} waiting',
    };
    const panel = mount({ copy });
    const engineering = station({
      station_id: 'engineering', station_name: 'Engineering', level: 'backfill', count: 0, demands: [],
    });
    panel.update(payload(
      station({ station_id: 'captain', station_name: 'Captain' }),
      engineering,
    ));

    expect(rowKeys()).toEqual(['ship-a/captain', 'ship-a/engineering']);
    expect(rowFor('ship-a/comms')).toBe(null);
    expect(rowFor('ship-a/navigation')).toBe(null);
    // One of the two rows drawn is a person, and the sentence says exactly
    // that: the vacated Comms and Navigation Stations are in nobody's total.
    expect(document.getElementById('gm-workload-status').textContent).toBe('Crewed Stations: 1');
    expect(document.getElementById('gm-workload-empty').hidden).toBe(true);
    expect(rowFor('ship-a/captain').querySelector('.gm-workload-count').textContent)
      .toBe('1 waiting');

    // Somebody sits down at Comms. Its System comes home, so the Station is a
    // row of its own again and the conversation moves with it — still exactly
    // one demand across the whole hull, never two.
    panel.update(payload(
      station({ station_id: 'captain', station_name: 'Captain', level: 'underused', count: 0, demands: [] }),
      station(),
      engineering,
    ));
    expect(rowKeys()).toEqual(['ship-a/captain', 'ship-a/comms', 'ship-a/engineering']);
    expect(rowFor('ship-a/comms').querySelector('.gm-workload-count').textContent)
      .toBe('1 waiting');
    expect(rowFor('ship-a/captain').querySelector('.gm-workload-count').textContent)
      .toBe('0 waiting');
    expect(document.getElementById('gm-workload-status').textContent).toBe('Crewed Stations: 2');
    panel.dispose();
  });

  it('gives the keyboard somewhere inside the panel to land when a Station leaves', () => {
    const panel = mount();
    panel.update(payload(
      station(),
      station({ station_id: 'helm', station_name: 'Helm', level: 'underused', count: 0, demands: [] }),
    ));
    const summary = rowFor('ship-a/helm').querySelector('summary');
    summary.click();
    summary.focus();
    expect(panel.isExpanded('ship-a/helm')).toBe(true);

    // The hull's Helm seat stops being projected — a despawn, a scenario change.
    panel.update(payload(station()));
    expect(rowKeys()).toEqual(['ship-a/comms']);
    expect(panel.isExpanded('ship-a/helm')).toBe(false);
    // Focus stays inside the region, on the sentence that just changed.
    expect(document.activeElement).toBe(document.getElementById('gm-workload-status'));
    expect(document.getElementById('gm-workload-status').tabIndex).toBe(-1);
    panel.dispose();
  });

  it('keeps the last honest advisory when a payload does not parse', () => {
    const panel = mount();
    panel.update(payload(station()));
    panel.update('not json');
    panel.update({ rows: [] });
    expect(rowKeys()).toEqual(['ship-a/comms']);
    expect(document.getElementById('gm-workload-empty').hidden).toBe(true);
    panel.reset();
    expect(rowKeys()).toEqual([]);
    expect(document.getElementById('gm-workload-empty').hidden).toBe(false);
    panel.dispose();
  });

  it('keeps its whole sentence and a reachable expander at 200% text', () => {
    // jsdom lays nothing out, so the two halves that CAN be checked here are:
    // the copy is not shortened for the larger size, and every control stays
    // keyboard-reachable. The measured half is the CSS contract below plus
    // tests/smoke/gm-layout.spec.js at 1280x720 with --a11y-text-scale: 2.
    document.documentElement.style.setProperty('--a11y-text-scale', '2');
    const panel = mount();
    panel.update(payload(station({ count: 3, level: 'overloaded', sustained_secs: 30 })));
    const row = rowFor('ship-a/comms');
    expect(row.querySelector('.gm-workload-name').textContent)
      .toBe('server.gm.workload.row');
    const summary = row.querySelector('summary');
    // A real <summary>, so it is in the tab ring and operable by Enter/Space
    // without the panel inventing a key handler.
    expect(summary.tagName).toBe('SUMMARY');
    summary.focus();
    expect(document.activeElement).toBe(summary);
    summary.click();
    expect(row.querySelector('details').open).toBe(true);
    expect(row.querySelector('.gm-workload-demands li').textContent)
      .toBe('server.gm.workload.reason.pending_comms');
    document.documentElement.style.removeProperty('--a11y-text-scale');
    panel.dispose();
  });

  it('has a row style that grows with the text rather than clipping it', () => {
    const css = readFileSync('gui/gm-workspace.css', 'utf8');
    const row = css.slice(css.indexOf('#gm-console.gm-desk #gm-workload-list > li {'));
    const rule = row.slice(0, row.indexOf('}'));
    expect(rule).toContain('overflow-wrap: anywhere');
    expect(rule).toContain('min-width: 0');
    // No fixed height to clip a wrapped Station sentence at 200%.
    expect(rule).not.toMatch(/(^|[^-])height:\s*\d/);
    // The expander wraps rather than squeezing, and keeps the shared hit floor.
    const summary = css.slice(css.indexOf('#gm-console.gm-desk #gm-workload-list summary {'));
    const summaryRule = summary.slice(0, summary.indexOf('}'));
    expect(summaryRule).toContain('flex-wrap: wrap');
    expect(summaryRule).toContain('min-height: var(--control-hit-min)');
  });
});
