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
import { describe, it, expect, beforeEach } from 'vitest';
import { t } from '../../gui/strings.js';
import { renderStation as battleshipRender } from '../../gui/battleship/tactical.console.js';
import { renderStation as cruiserRender } from '../../gui/cruiser/tactical.console.js';
import { renderStation as destroyerRender } from '../../gui/destroyer/tactical.console.js';
import { renderStation as courierRender } from '../../gui/courier/tactical.console.js';

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
    '<ph-station-damage id="station-damage"></ph-station-damage>' +
    '<span id="footer-target"></span>' +
    '<span id="tactical-auto-badge" hidden></span>',
  cruiser:
    '<ph-tactical-radar id="tactical-radar"></ph-tactical-radar>' +
    '<ph-phasers-controls id="phasers-controls"></ph-phasers-controls>' +
    '<ph-torpedo-controls id="torpedo-controls"></ph-torpedo-controls>' +
    '<ph-station-damage id="station-damage"></ph-station-damage>' +
    '<span id="footer-target"></span>' +
    '<span id="tactical-auto-badge" hidden></span>',
  destroyer:
    '<ph-tactical-radar id="tactical-radar"></ph-tactical-radar>' +
    '<ph-phasers-controls id="phasers-controls"></ph-phasers-controls>' +
    '<ph-blasters-controls id="blasters-controls"></ph-blasters-controls>' +
    '<ph-torpedo-controls id="torpedo-controls"></ph-torpedo-controls>' +
    '<ph-station-damage id="station-damage"></ph-station-damage>' +
    '<span id="footer-target"></span>' +
    '<span id="tactical-auto-badge" hidden></span>' +
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
    '<ph-boost-btn id="boost"></ph-boost-btn>' +
    '<ph-station-damage id="damage"></ph-station-damage>',
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
      payload: { blips: [{ uuid: 'cr-1' }], target_uuid: 'cr-1' },
      uuid: 'cr-1',
    },
    {
      hull: 'destroyer',
      render: destroyerRender,
      // keyed payload (weaponsView is systemView)
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
      const cleared = c.hull === 'battleship' || c.hull === 'cruiser'
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
    expect(el('station-damage').state).toEqual({ pct: 0.8 });
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

// ── Cruiser: the `var t` shadowing regression ────────────────────────────────
describe('cruiser tactical renderStation', () => {
  beforeEach(() => mount(FIXTURES.cruiser));

  it('renders a LOCKED footer for an unnamed lock without throwing (old var-t shadow bug)', () => {
    // The old inline render did `var t = getElementById('torpedo-controls')`,
    // shadowing the String Table t(); `t('console.common.locked')` then threw
    // because a DOM element is not callable. This must resolve to real text.
    expect(() => cruiserRender({ blips: [{ uuid: 'e2' }], target_uuid: 'e2' }, document)).not.toThrow();
    expect(el('footer-target').textContent).toBe(t('console.common.locked'));
  });

  it('shows the target name when one is present', () => {
    cruiserRender({ blips: [{ uuid: 'e3' }], target_uuid: 'e3', target_name: 'Corsair' }, document);
    expect(el('footer-target').textContent).toBe('Corsair');
  });

  it('carries no blaster panel and still renders the core', () => {
    cruiserRender({ blips: [{ uuid: 'e4' }], target_uuid: 'e4', banks: [{ id: 'p' }], own_hull: { pct: 0.5 } }, document);
    expect(el('blasters-controls')).toBeNull();
    expect(el('phasers-controls').state.target_valid).toBe(true);
    expect(el('station-damage').state).toEqual({ pct: 0.5 });
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

  it('reads the weapons view via systemView and keeps the blaster column visible', () => {
    destroyerRender(payload, document);
    expect(el('blasters-controls').state).toEqual({ banks: [{ id: 'bp' }] });
    expect(el('blasters-controls').hidden).toBe(false);
    expect(el('station-damage').state).toEqual({ pct: 1 });
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
    expect(el('damage').state).toEqual({ pct: 0.5 });
  });

  it('carries no phaser / torpedo / footer / auto-badge panel', () => {
    courierRender(payload, document);
    expect(el('phasers-controls')).toBeNull();
    expect(el('torpedo-controls')).toBeNull();
    expect(el('footer-target')).toBeNull();
    expect(el('tactical-auto-badge')).toBeNull();
  });
});
