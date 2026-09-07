// @vitest-environment jsdom
/**
 * tests/client/tactical-console.test.js — the four Tactical consoles on one
 * renderer (issue #1234, T4.C2).
 *
 * Each hull's `.html` imports its `renderStation` from
 * `gui/<class>/tactical.console.js`; this suite imports the SAME functions and
 * drives them against a jsdom fixture, so the radar contract is asserted per
 * hull without a browser. The load-bearing assertion — the bug this seam
 * removes by construction — is that ALL FOUR hulls set BOTH `target_uuid` (the
 * inner `ph-radar`'s locked contact) and `selected_target_uuid` (the outer
 * highlight ring) on `ph-tactical-radar`. Before #1234 only the battleship did.
 *
 * The custom-element modules are deliberately NOT imported: an un-upgraded
 * `<ph-*>` element is a plain `HTMLUnknownElement`, so assigning `.state`
 * stores a readable property and we assert the exact object the console pushed,
 * with no shadow-DOM / canvas / ResizeObserver machinery in the way.
 */
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { t } from '../../gui/strings.js';
import { renderStation as battleshipRender } from '../../gui/battleship/tactical.console.js';
import { renderStation as rawCruiserRender } from '../../gui/cruiser/tactical.console.js';
import { renderStation as rawDestroyerRender } from '../../gui/destroyer/tactical.console.js';
import { renderStation as rawCourierRender } from '../../gui/courier/tactical.console.js';
import { withConsoleFamilyProjection } from './console-family-fixture.js';

// The cruiser joined the keyed hulls in issue #1389: authoring a Security
// System on Tactical made that seat span two Console Families, so the host
// builds it the system-id-keyed payload and its weapons view is selected by
// family like the destroyer's.
const cruiserRender = (payload, doc) => rawCruiserRender(withConsoleFamilyProjection(payload), doc);
const destroyerRender = (payload, doc) => rawDestroyerRender(withConsoleFamilyProjection(payload), doc);
const courierRender = (payload, doc) => rawCourierRender(withConsoleFamilyProjection(payload), doc);

function mount(markup) {
  document.body.innerHTML = markup;
}
const el = (id) => document.getElementById(id);

// ── Fixtures: only the elements each hull's markup actually carries ──────────
const FIXTURES = {
  battleship:
    '<ph-tactical-radar id="tactical-radar"></ph-tactical-radar>' +
    '<ph-phasers-controls id="phasers-controls"></ph-phasers-controls>' +
    '<ph-blasters-controls id="blasters-controls" hidden></ph-blasters-controls>' +
    '<ph-torpedo-controls id="torpedo-controls"></ph-torpedo-controls>' +
    '<span id="footer-target"></span>' +
    '<span id="tactical-auto-badge" hidden></span>',
  cruiser:
    '<ph-tactical-radar id="tactical-radar"></ph-tactical-radar>' +
    '<ph-phasers-controls id="phasers-controls"></ph-phasers-controls>' +
    '<ph-torpedo-controls id="torpedo-controls"></ph-torpedo-controls>' +
    '<span id="footer-target"></span>' +
    '<span id="tactical-auto-badge" hidden></span>' +
    '<ph-security-teams id="security-teams"></ph-security-teams>' +
    '<ph-target-lock-card id="target-lock-card"></ph-target-lock-card>' +
    '<ph-dossier-panel id="dossier-panel"></ph-dossier-panel>',
  destroyer:
    '<ph-tactical-radar id="tactical-radar"></ph-tactical-radar>' +
    '<ph-phasers-controls id="phasers-controls"></ph-phasers-controls>' +
    '<ph-blasters-controls id="blasters-controls"></ph-blasters-controls>' +
    '<ph-torpedo-controls id="torpedo-controls"></ph-torpedo-controls>' +
    '<span id="footer-target"></span>' +
    '<span id="tactical-auto-badge" hidden></span>' +
    '<ph-target-lock-card id="target-lock-card"></ph-target-lock-card>' +
    '<ph-dossier-panel id="dossier-panel"></ph-dossier-panel>' +
    '<div id="command-advice" hidden><span id="command-advice-stance"></span></div>',
  courier:
    '<ph-tactical-radar id="tactical-radar"></ph-tactical-radar>' +
    '<ph-blasters-controls id="blasters"></ph-blasters-controls>' +
    '<ph-sensor-radar id="sensor-radar"></ph-sensor-radar>' +
    '<ph-sensor-panel id="sensor-panel"></ph-sensor-panel>' +
    '<ph-helm-joystick id="helm"></ph-helm-joystick>' +
    '<ph-lateral-thrust-joystick id="lateral"></ph-lateral-thrust-joystick>' +
    '<ph-impulse-btn id="impulse"></ph-impulse-btn>' +
    '<ph-boost-btn id="boost"></ph-boost-btn>',
};

// ── The one contract that used to diverge: both radar uuids, all four hulls ──
describe('inner-radar target contract — both uuids set on all four hulls (#1234)', () => {
  const cases = [
    {
      hull: 'battleship',
      render: battleshipRender,
      // flat payload (weaponsView is identity)
      payload: { blips: [{ uuid: 'bs-1' }], target_uuid: 'bs-1' },
      uuid: 'bs-1',
    },
    {
      hull: 'cruiser',
      render: cruiserRender,
      payload: { systems: { 'tactical-radar': { blips: [{ uuid: 'cr-1' }], target_uuid: 'cr-1' } } },
      uuid: 'cr-1',
    },
    {
      hull: 'destroyer',
      render: destroyerRender,
      // keyed payload (weaponsView is selected by Console Family)
      payload: { systems: { 'tactical-radar': { blips: [{ uuid: 'ds-1' }], target_uuid: 'ds-1' } } },
      uuid: 'ds-1',
    },
    {
      hull: 'courier',
      render: courierRender,
      payload: { systems: { 'tactical-radar': { blips: [{ uuid: 'co-1' }], target_uuid: 'co-1' } } },
      uuid: 'co-1',
    },
  ];

  for (const c of cases) {
    it(`${c.hull}: ph-tactical-radar receives target_uuid AND selected_target_uuid`, () => {
      mount(FIXTURES[c.hull]);
      c.render(c.payload, document);
      const radar = el('tactical-radar').state;
      expect(radar.target_uuid).toBe(c.uuid);
      expect(radar.selected_target_uuid).toBe(c.uuid);
    });

    it(`${c.hull}: both radar uuids are null when nothing is locked`, () => {
      mount(FIXTURES[c.hull]);
      const cleared = c.hull === 'battleship'
        ? { blips: [] }
        : { systems: { 'tactical-radar': { blips: [] } } };
      c.render(cleared, document);
      const radar = el('tactical-radar').state;
      expect(radar.target_uuid).toBeNull();
      expect(radar.selected_target_uuid).toBeNull();
    });
  }
});

// ── Battleship: the reference hull ───────────────────────────────────────────
describe('battleship tactical renderStation', () => {
  beforeEach(() => mount(FIXTURES.battleship));

  const base = {
    blips: [{ uuid: 'enemy-1', radar_x: 0.2, radar_y: 0.1 }],
    banks: [{ id: 'fore' }],
    tubes: [{ id: 't1' }],
    torpedo_count: 4, torpedo_max: 12,
    phaser_arcs: [{ facing_deg: 0, arc_deg: 30 }],
    ship_x: 10, ship_z: 20, ship_speed: 5, ship_heading: 90,
    target_uuid: 'enemy-1', target_name: 'Raider',
    phaser_mode: 'Manual', tactical_auto: true,
    blasters: [],
    own_hull: { pct: 0.8 },
  };

  it('drives every panel from the flat payload', () => {
    battleshipRender(base, document);
    const radar = el('tactical-radar').state;
    expect(radar.blips).toEqual(base.blips);
    expect(radar.phaser_arcs).toEqual(base.phaser_arcs);
    expect(el('phasers-controls').state).toEqual({ banks: [{ id: 'fore' }], target_valid: true, mode: 'Manual' });
    expect(el('torpedo-controls').state).toEqual({ tubes: [{ id: 't1' }], magazine: { current: 4, max: 12 }, target_uuid: 'enemy-1' });
    expect(el('footer-target').textContent).toBe('Raider');
  });

  it('hides the blaster panel when empty and shows it when banks arrive (issue #925)', () => {
    battleshipRender(base, document);
    expect(el('blasters-controls').hidden).toBe(true);
    battleshipRender({ ...base, blasters: [{ id: 'b1' }] }, document);
    expect(el('blasters-controls').hidden).toBe(false);
    expect(el('blasters-controls').state).toEqual({ banks: [{ id: 'b1' }] });
  });

  it('falls back to torpedo max default 20 when the wire sends no magazine size', () => {
    battleshipRender({ ...base, torpedo_max: undefined, torpedo_count: 0 }, document);
    expect(el('torpedo-controls').state.magazine.max).toBe(20);
  });

  it('shows LOCKED for an unnamed lock and NO TARGET when clear', () => {
    battleshipRender({ ...base, target_name: undefined }, document);
    expect(el('footer-target').textContent).toBe(t('console.common.locked'));
    battleshipRender({ blips: [] }, document);
    expect(el('footer-target').textContent).toBe(t('console.common.no_target'));
  });

  it('reflects the AUTO badge', () => {
    battleshipRender(base, document);
    expect(el('tactical-auto-badge').hidden).toBe(false);
    battleshipRender({ ...base, tactical_auto: false }, document);
    expect(el('tactical-auto-badge').hidden).toBe(true);
  });
});

// ── Cruiser: the `var t` shadowing regression + the Security/target-card/
//    Intel tails (#1389, #1393) ──────────────────────────────────────────────
describe('cruiser tactical renderStation', () => {
  beforeEach(() => mount(FIXTURES.cruiser));

  /** A keyed cruiser payload: one Tactical view, optionally a Security one. */
  const keyed = (weapons, extra = {}) => ({
    systems: { 'tactical-radar': weapons, ...(extra.systems || {}) },
    ...Object.fromEntries(Object.entries(extra).filter(([k]) => k !== 'systems')),
  });

  it('renders a LOCKED footer for an unnamed lock without throwing (old var-t shadow bug)', () => {
    // The old inline render did `var t = getElementById('torpedo-controls')`,
    // shadowing the String Table t(); `t('console.common.locked')` then threw
    // because a DOM element is not callable. This must resolve to real text.
    expect(() => cruiserRender(keyed({ blips: [{ uuid: 'e2' }], target_uuid: 'e2' }), document)).not.toThrow();
    expect(el('footer-target').textContent).toBe(t('console.common.locked'));
  });

  it('shows the target name when one is present', () => {
    cruiserRender(keyed({ blips: [{ uuid: 'e3' }], target_uuid: 'e3', target_name: 'Corsair' }), document);
    expect(el('footer-target').textContent).toBe('Corsair');
  });

  it('carries no blaster panel and still renders the core', () => {
    cruiserRender(keyed({ blips: [{ uuid: 'e4' }], target_uuid: 'e4', banks: [{ id: 'p' }] }, { own_hull: { pct: 0.5 } }), document);
    expect(el('blasters-controls')).toBeNull();
    expect(el('phasers-controls').state.target_valid).toBe(true);
  });

  // ── Security (issue #1389) ─────────────────────────────────────────────
  // Tactical owns the Security System on this hull, so its published view
  // arrives under this station's payload keyed by the system's own id, and the
  // tail feeds the panel the SEC tab shows — whole, with nothing re-derived.
  it('feeds the Security panel the published Security-family view', () => {
    const security = {
      system_id: 'security',
      range: 400,
      teams: [{ teamIdx: 0, state: 'available' }],
      targets: [{ uuid: 'tether-head', in_range: true, actions: [{ id: 'assist_evacuation' }] }],
      refusal: null,
    };
    cruiserRender(
      keyed({ blips: [], target_uuid: null }, { systems: { security } }),
      document,
    );
    expect(el('security-teams').state).toEqual(security);
  });

  it('gives the Security panel an empty view when the hull publishes none', () => {
    cruiserRender(keyed({ blips: [], target_uuid: null }), document);
    expect(el('security-teams').state).toEqual({});
  });

  it('still renders the weapons panels while a Security view rides alongside', () => {
    cruiserRender(
      keyed(
        { blips: [{ uuid: 'e5' }], target_uuid: 'e5', banks: [{ id: 'fore' }], tubes: [{ id: 't1' }] },
        { systems: { security: { teams: [], targets: [] } } },
      ),
      document,
    );
    expect(el('tactical-radar').state.target_uuid).toBe('e5');
    expect(el('phasers-controls').state.banks).toEqual([{ id: 'fore' }]);
    expect(el('torpedo-controls').state.tubes).toEqual([{ id: 't1' }]);
  });

  // ── Target lock card (issue #1393) ──────────────────────────────────────
  it('feeds the target lock card the Tactical lock facts, beside the footer', () => {
    const withFacts = {
      blips: [{ uuid: 'e6' }], target_uuid: 'e6',
      target_name: 'Raider', target_stance: 'hostile', target_class: 'Corvette',
      target_bearing: 12, target_range: 88, target_hull_pct: 55,
      target_shields: [{ label: 'fore', hp: 50, max_hp: 100 }], target_shield_freq: 0.3,
    };
    cruiserRender(keyed(withFacts), document);
    expect(el('target-lock-card').state).toEqual({
      target_uuid: 'e6',
      target_name: 'Raider',
      target_stance: 'hostile',
      target_class: 'Corvette',
      target_bearing: 12,
      target_range: 88,
      target_hull_pct: 55,
      target_shields: [{ label: 'fore', hp: 50, max_hp: 100 }],
      target_shield_freq: 0.3,
    });
  });

  it('clears the target lock card to its no-target defaults when nothing is locked', () => {
    cruiserRender(keyed({ blips: [] }), document);
    expect(el('target-lock-card').state).toEqual({
      target_uuid: null,
      target_name: null,
      target_stance: null,
      target_class: null,
      target_bearing: null,
      target_range: null,
      target_hull_pct: null,
      target_shields: [],
      target_shield_freq: null,
    });
  });
});

// ── Cruiser: the Intel tab's unread badge (issue #1393) ──────────────────────
//
// Mirrors the destroyer's own suite below (issue #1373): the count the shell's
// Station Bar draws is computed here, in the console's own render, because
// only the console knows whether the seat is LOOKING at the panel. This hull
// has no Command-intent strip, so its overlay fixture carries only the two
// tabbed panels this seat declares (Security, Intel).

describe('cruiser tactical intel badge', () => {
  const OVERLAYS =
    '<div class="overlay-panel" id="security-overlay"></div>' +
    '<div class="overlay-panel" id="intel-overlay"></div>';

  let reported;
  // A FRESH document per test, not the shared jsdom one — see the destroyer
  // suite's own comment on why: the "already read" baseline lives in a
  // WeakMap keyed on the document (gui/cruiser/tactical.console.js).
  let doc;

  const freshDoc = () => {
    const made = document.implementation.createHTMLDocument();
    made.body.innerHTML = FIXTURES.cruiser + OVERLAYS;
    return made;
  };

  const subject = (uuid, facts) => ({
    uuid,
    facts: Array.from({ length: facts }, (_, i) => ({ text: 'f' + i })),
    evidence: [],
  });
  const payloadWith = (dossiers) => ({
    systems: { 'tactical-radar': { blips: [], banks: [], tubes: [] } },
    dossiers,
  });
  const openIntel = (target = doc) =>
    target.getElementById('intel-overlay').classList.add('open');
  const closeIntel = (target = doc) =>
    target.getElementById('intel-overlay').classList.remove('open');
  const latestBadge = () => reported.filter(([id]) => id === 'intel-overlay').at(-1)?.[1];

  beforeEach(() => {
    doc = freshDoc();
    reported = [];
    window.__setConsoleTabBadge = (id, count) => { reported.push([id, count]); };
  });

  afterEach(() => { delete window.__setConsoleTabBadge; });

  it('reports every subject with something on file to a seat that has not looked', () => {
    cruiserRender(payloadWith([subject('a', 1), subject('b', 2)]), doc);
    expect(latestBadge()).toBe(2);
  });

  it('grows when a dossier gains a fact while the panel is closed', () => {
    cruiserRender(payloadWith([subject('a', 1)]), doc);
    openIntel();
    cruiserRender(payloadWith([subject('a', 1)]), doc);
    expect(latestBadge()).toBe(0);

    closeIntel();
    cruiserRender(payloadWith([subject('a', 2)]), doc);
    expect(latestBadge()).toBe(1);
  });

  it('clears the moment the panel is open — reading is what marks it read', () => {
    cruiserRender(payloadWith([subject('a', 3), subject('b', 1)]), doc);
    expect(latestBadge()).toBe(2);
    openIntel();
    cruiserRender(payloadWith([subject('a', 3), subject('b', 1)]), doc);
    expect(latestBadge()).toBe(0);
  });

  it('stays cleared over repeated renders with the panel open', () => {
    openIntel();
    cruiserRender(payloadWith([subject('a', 3)]), doc);
    cruiserRender(payloadWith([subject('a', 3)]), doc);
    expect(reported.map(([, count]) => count)).toEqual([0, 0]);
  });

  it('reports nothing unread for a hull with no dossiers at all', () => {
    cruiserRender(payloadWith(undefined), doc);
    expect(latestBadge()).toBe(0);
  });

  it('renders fine on a console whose shell installed no badge hook', () => {
    delete window.__setConsoleTabBadge;
    expect(() => cruiserRender(payloadWith([subject('a', 1)]), doc)).not.toThrow();
  });

  it('starts a second document from an empty baseline, whatever the first read', () => {
    const first = freshDoc();
    openIntel(first);
    cruiserRender(payloadWith([subject('a', 2), subject('b', 1)]), first);
    expect(latestBadge()).toBe(0);

    const second = freshDoc();
    cruiserRender(payloadWith([subject('a', 2), subject('b', 1)]), second);
    expect(latestBadge()).toBe(2);
  });
});

// ── Destroyer: keyed payload + dossier / Command-intent tail ──────────────────
describe('destroyer tactical renderStation', () => {
  beforeEach(() => mount(FIXTURES.destroyer));

  const w = { blips: [{ uuid: 'd1' }], banks: [], blasters: [{ id: 'bp' }], tubes: [], target_uuid: 'd1', tactical_auto: false };
  const payload = {
    systems: { 'tactical-radar': w },
    own_hull: { pct: 1 },
    dossiers: [{ id: 'x' }],
    command_advice: { stance_label: 'console.common.auto', stance_id: 'aggressive' },
  };

  it('reads the weapons view via projected Console Family and keeps the blaster column visible', () => {
    destroyerRender(payload, document);
    expect(el('blasters-controls').state).toEqual({ banks: [{ id: 'bp' }] });
    expect(el('blasters-controls').hidden).toBe(false);
  });

  it('renders the intel dossier tail', () => {
    destroyerRender(payload, document);
    expect(el('dossier-panel').state).toEqual({ dossiers: [{ id: 'x' }] });
  });

  it('shows Command-intent advice when present and hides it when absent', () => {
    destroyerRender(payload, document);
    expect(el('command-advice').hidden).toBe(false);
    expect(el('command-advice-stance').textContent).toBe(t('console.common.auto'));
    destroyerRender({ ...payload, command_advice: null }, document);
    expect(el('command-advice').hidden).toBe(true);
  });

  it('prefixes the footer target and falls back to the uuid when unnamed', () => {
    destroyerRender(payload, document);
    expect(el('footer-target').textContent).toBe('◉ d1');
  });

  // ── Target lock card (issue #1378) ──────────────────────────────────────
  it('feeds the target lock card the Tactical lock facts, beside the footer', () => {
    const withFacts = {
      ...w,
      target_name: 'Raider', target_stance: 'hostile', target_class: 'Corvette',
      target_bearing: 12, target_range: 88, target_hull_pct: 55,
      target_shields: [{ label: 'fore', hp: 50, max_hp: 100 }], target_shield_freq: 0.3,
    };
    destroyerRender({ ...payload, systems: { 'tactical-radar': withFacts } }, document);
    expect(el('target-lock-card').state).toEqual({
      target_uuid: 'd1',
      target_name: 'Raider',
      target_stance: 'hostile',
      target_class: 'Corvette',
      target_bearing: 12,
      target_range: 88,
      target_hull_pct: 55,
      target_shields: [{ label: 'fore', hp: 50, max_hp: 100 }],
      target_shield_freq: 0.3,
    });
    // The footer stays alongside it, unchanged.
    expect(el('footer-target').textContent).toBe('◉ Raider');
  });

  it('clears the target lock card to its no-target defaults when nothing is locked', () => {
    destroyerRender({ ...payload, systems: { 'tactical-radar': { blips: [] } } }, document);
    expect(el('target-lock-card').state).toEqual({
      target_uuid: null,
      target_name: null,
      target_stance: null,
      target_class: null,
      target_bearing: null,
      target_range: null,
      target_hull_pct: null,
      target_shields: [],
      target_shield_freq: null,
    });
  });
});

// ── Destroyer: the Intel tab's unread badge (issue #1373) ────────────────────
//
// The count the shell's Station Bar draws is computed here, in the console's
// own render, because only the console knows whether the seat is LOOKING at
// the panel. gui/intel-unread.js's own suite pins the arithmetic; this pins
// that the destroyer console feeds it the right subjects, reads the panel's
// open state, and reports through `__setConsoleTabBadge`.

describe('destroyer tactical intel badge', () => {
  const OVERLAYS =
    '<div class="overlay-panel" id="security-overlay"></div>' +
    '<div class="overlay-panel" id="intel-overlay"></div>';

  let reported;
  // A FRESH document per test, not the shared jsdom one. The console holds the
  // "already read" baseline in a WeakMap keyed on the document (see
  // gui/destroyer/tactical.console.js), and `mount()` only swaps
  // `document.body.innerHTML` — the document object itself survives every
  // `beforeEach`, so a baseline left by an earlier test would still be there
  // and these tests would be asserting each other's leftovers rather than what
  // they say. A new document is also the production story: a reloaded iframe
  // is a new document and starts from an empty baseline.
  let doc;

  const freshDoc = () => {
    const made = document.implementation.createHTMLDocument();
    made.body.innerHTML = FIXTURES.destroyer + OVERLAYS;
    return made;
  };

  const subject = (uuid, facts) => ({
    uuid,
    facts: Array.from({ length: facts }, (_, i) => ({ text: 'f' + i })),
    evidence: [],
  });
  const payloadWith = (dossiers) => ({
    systems: { 'tactical-radar': { blips: [], banks: [], blasters: [], tubes: [] } },
    own_hull: { pct: 1 },
    dossiers,
  });
  const openIntel = (target = doc) =>
    target.getElementById('intel-overlay').classList.add('open');
  const closeIntel = (target = doc) =>
    target.getElementById('intel-overlay').classList.remove('open');
  const latestBadge = () => reported.filter(([id]) => id === 'intel-overlay').at(-1)?.[1];

  beforeEach(() => {
    doc = freshDoc();
    reported = [];
    window.__setConsoleTabBadge = (id, count) => { reported.push([id, count]); };
  });

  afterEach(() => { delete window.__setConsoleTabBadge; });

  it('reports every subject with something on file to a seat that has not looked', () => {
    destroyerRender(payloadWith([subject('a', 1), subject('b', 2)]), doc);
    expect(latestBadge()).toBe(2);
  });

  it('grows when a dossier gains a fact while the panel is closed', () => {
    destroyerRender(payloadWith([subject('a', 1)]), doc);
    openIntel();
    destroyerRender(payloadWith([subject('a', 1)]), doc);
    expect(latestBadge()).toBe(0);

    closeIntel();
    destroyerRender(payloadWith([subject('a', 2)]), doc);
    expect(latestBadge()).toBe(1);
  });

  it('clears the moment the panel is open — reading is what marks it read', () => {
    destroyerRender(payloadWith([subject('a', 3), subject('b', 1)]), doc);
    expect(latestBadge()).toBe(2);
    openIntel();
    destroyerRender(payloadWith([subject('a', 3), subject('b', 1)]), doc);
    expect(latestBadge()).toBe(0);
  });

  it('stays cleared over repeated renders with the panel open', () => {
    openIntel();
    destroyerRender(payloadWith([subject('a', 3)]), doc);
    destroyerRender(payloadWith([subject('a', 3)]), doc);
    expect(reported.map(([, count]) => count)).toEqual([0, 0]);
  });

  it('reports nothing unread for a hull with no dossiers at all', () => {
    destroyerRender(payloadWith(undefined), doc);
    expect(latestBadge()).toBe(0);
  });

  it('renders fine on a console whose shell installed no badge hook', () => {
    delete window.__setConsoleTabBadge;
    expect(() => destroyerRender(payloadWith([subject('a', 1)]), doc)).not.toThrow();
  });

  it('starts a second document from an empty baseline, whatever the first read', () => {
    // The isolation the suite above depends on, asserted rather than assumed —
    // and the reload story itself: one document reads the files, a second one
    // (the reloaded iframe, the next seat) is told about all of them again.
    const first = freshDoc();
    openIntel(first);
    destroyerRender(payloadWith([subject('a', 2), subject('b', 1)]), first);
    expect(latestBadge()).toBe(0);

    const second = freshDoc();
    destroyerRender(payloadWith([subject('a', 2), subject('b', 1)]), second);
    expect(latestBadge()).toBe(2);
  });
});

// ── Courier: keyed payload + sensors / helm tail ─────────────────────────────
describe('courier tactical renderStation', () => {
  beforeEach(() => mount(FIXTURES.courier));

  const weapons = { blips: [{ uuid: 'c1' }], blasters: [{ id: 'bf' }], target_uuid: 'c1' };
  const sensors = { blips: [{ uuid: 's1' }], contacts: [] };
  const helm = { helm_auto: true, lateral_auto: false, impulse_charge_progress: 0.5, boost_enabled: true, boost_active: false, boost_battery: 0.75 };
  const payload = {
    systems: { 'tactical-radar': weapons, sensors, 'helm-thrust': helm },
    own_hull: { pct: 0.5 },
  };

  it('drives radar + blasters from the weapons view', () => {
    courierRender(payload, document);
    expect(el('tactical-radar').state.target_uuid).toBe('c1');
    expect(el('blasters').state).toEqual({ banks: [{ id: 'bf' }] });
  });

  it('drives the sensors and helm tail', () => {
    courierRender(payload, document);
    expect(el('sensor-radar').state).toEqual(sensors);
    expect(el('sensor-panel').state).toEqual(sensors);
    expect(el('helm').state).toEqual({ auto: true });
    expect(el('lateral').state).toEqual({ auto: false });
    expect(el('impulse').state).toEqual({ state: 'charging', charge_pct: 50, auto: true });
    expect(el('boost').state).toEqual({ available: true, active: false, recharge_pct: 75, auto: true });
  });

  it('carries no phaser / torpedo / footer / auto-badge panel', () => {
    courierRender(payload, document);
    expect(el('phasers-controls')).toBeNull();
    expect(el('torpedo-controls')).toBeNull();
    expect(el('footer-target')).toBeNull();
    expect(el('tactical-auto-badge')).toBeNull();
  });
});
