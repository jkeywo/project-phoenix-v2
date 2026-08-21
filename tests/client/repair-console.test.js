// @vitest-environment jsdom
/**
 * tests/client/repair-console.test.js — the battleship's Repair console on
 * the shared renderer (issue #1235, T4.C3 chunk 1).
 *
 * `gui/battleship/repair.html` imports `renderStation` from
 * `gui/battleship/repair.console.js`; this suite imports the SAME function
 * and drives it against a jsdom fixture, so the contract — including the
 * External repair-team dispatch panel (issue #1161) — is asserted without a
 * browser. Only the battleship stations a dedicated Repair console today (the
 * destroyer's equivalent panel lives on its Engineering seat — see
 * engineering-console.test.js), so this suite is single-hull, matching
 * gui/stations/repair-console.js's own "one hull today, N later" framing.
 */
import { describe, it, expect, beforeEach } from 'vitest';
import { t } from '../../gui/strings.js';
import { renderStation } from '../../gui/battleship/repair.console.js';

function mount(markup) {
  document.body.innerHTML = markup;
}
const el = (id) => document.getElementById(id);

const FIXTURE =
  '<ph-hull-integrity id="hull-integrity"></ph-hull-integrity>' +
  '<ph-station-damage id="core-damage"></ph-station-damage>' +
  '<ph-repair-teams id="repair-teams"></ph-repair-teams>' +
  '<ph-station-damage id="station-damage"></ph-station-damage>' +
  '<span id="footer-right"></span>' +
  '<span id="repair-auto-badge" hidden></span>' +
  '<div id="dispatch-panel" hidden>' +
    '<button id="dispatch-btn" class="dispatch-btn"></button>' +
    '<span id="dispatch-status"></span>' +
    '<div id="dispatch-refusal" hidden></div>' +
  '</div>';

describe('battleship repair renderStation', () => {
  beforeEach(() => mount(FIXTURE));

  const payload = {
    overall_hull: { pct: 0.55, destroyed_pct: 0.05 },
    core_systems: [{ id: 'core-1' }],
    teams: [{ id: 't1', status: 'idle' }, { id: 't2', status: 'repairing' }],
    repair_auto: true,
    dispatch_targets: [{ id: 'dt1' }],
    damaged_systems: [{ id: 'ds1' }],
    own_hull: { pct: 0.8 },
  };

  it('drives hull integrity, core damage, repair teams and station-damage', () => {
    renderStation(payload, document);
    expect(el('hull-integrity').state).toEqual({ total_pct: 0.55, destroyed_pct: 0.05 });
    expect(el('core-damage').state).toEqual({ entries: [{ id: 'core-1' }] });
    expect(el('repair-teams').state).toEqual({ teams: payload.teams, auto: true, targets: [{ id: 'dt1' }], damaged: [{ id: 'ds1' }] });
    expect(el('station-damage').state).toEqual({ pct: 0.8 });
  });

  it('renders the localized footer status with active/total team counts', () => {
    renderStation(payload, document);
    expect(el('footer-right').textContent).toBe(
      t('console.repair.footer_status', { pct: 55, active: 1, total: 2 }),
    );
  });

  it('reflects the AUTO badge', () => {
    renderStation(payload, document);
    expect(el('repair-auto-badge').hidden).toBe(false);
    renderStation({ ...payload, repair_auto: false }, document);
    expect(el('repair-auto-badge').hidden).toBe(true);
  });

  it('hides the dispatch panel when the payload carries no external_dispatch', () => {
    renderStation(payload, document);
    expect(el('dispatch-panel').hidden).toBe(true);
  });

  it('shows the dispatch panel and toggles dispatch/recall text+class', () => {
    const idle = { ...payload, external_dispatch: { target: null, range: 300 } };
    renderStation(idle, document);
    expect(el('dispatch-panel').hidden).toBe(false);
    expect(el('dispatch-btn').classList.contains('working')).toBe(false);
    expect(el('dispatch-btn').textContent).toBe(t('console.repair.dispatch.send'));
    expect(el('dispatch-status').textContent).toContain(t('console.repair.dispatch.idle'));

    const working = { ...payload, external_dispatch: { target: 'x', target_name: 'console.repair.dispatch.idle' } };
    renderStation(working, document);
    expect(el('dispatch-btn').classList.contains('working')).toBe(true);
    expect(el('dispatch-btn').textContent).toBe(t('console.repair.dispatch.recall'));
  });

  it('shows a dispatch refusal when present and clears it when absent', () => {
    const refused = { ...payload, external_dispatch: { target: null, refusal: 'console.repair.dispatch.idle' } };
    renderStation(refused, document);
    expect(el('dispatch-refusal').hidden).toBe(false);
    const clear = { ...payload, external_dispatch: { target: null } };
    renderStation(clear, document);
    expect(el('dispatch-refusal').hidden).toBe(true);
  });

  it('falls back to a full hull when the payload carries none', () => {
    renderStation({ teams: [] }, document);
    expect(el('hull-integrity').state).toEqual({ total_pct: 1, destroyed_pct: undefined });
  });
});
