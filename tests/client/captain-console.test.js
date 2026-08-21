// @vitest-environment jsdom
/**
 * tests/client/captain-console.test.js — the four Captain consoles on one
 * renderer (issue #1235, T4.C3 chunk 3 — final chunk of the console-seam
 * programme).
 *
 * Each hull's `.html` imports its `renderStation` from
 * `gui/<class>/captain.console.js`; this suite imports the SAME functions
 * and drives them against a jsdom fixture, so the camera/red-alert/
 * objectives/station-damage contract is asserted per hull without a browser.
 *
 * The custom-element modules are deliberately NOT imported: an un-upgraded
 * `<ph-*>` element is a plain `HTMLUnknownElement`, so assigning `.state`
 * stores a readable property and we assert the exact object the console
 * pushed, with no shadow-DOM machinery in the way.
 */
import { describe, it, expect, beforeEach } from 'vitest';
import { t } from '../../gui/strings.js';
import { renderStation as battleshipRender } from '../../gui/battleship/captain.console.js';
import { renderStation as cruiserRender } from '../../gui/cruiser/captain.console.js';
import { renderStation as destroyerRender } from '../../gui/destroyer/captain.console.js';
import { renderStation as courierRender } from '../../gui/courier/captain.console.js';

function mount(markup) {
  document.body.innerHTML = markup;
}
const el = (id) => document.getElementById(id);

// ── Fixtures: only the elements each hull's markup actually carries ──────────
const FIXTURES = {
  battleship:
    '<ph-camera-select id="camera-select"></ph-camera-select>' +
    '<ph-red-alert id="red-alert"></ph-red-alert>' +
    '<ph-objective-list id="objective-list"></ph-objective-list>' +
    '<ph-station-damage id="station-damage"></ph-station-damage>' +
    '<span id="footer-target"></span>' +
    '<span id="captain-auto-badge" hidden></span>' +
    '<div id="objectives" hidden></div>' +
    '<div id="dir" hidden></div>' +
    '<div id="alert" hidden></div>',
  cruiser:
    '<ph-camera-select id="camera-select"></ph-camera-select>' +
    '<ph-red-alert id="red-alert"></ph-red-alert>' +
    '<ph-objective-list id="objective-list"></ph-objective-list>' +
    '<ph-station-damage id="station-damage"></ph-station-damage>' +
    '<span id="footer-target"></span>',
  destroyer:
    '<ph-camera-select id="camera-select"></ph-camera-select>' +
    '<ph-red-alert id="red-alert"></ph-red-alert>' +
    '<ph-objective-list id="objective-list"></ph-objective-list>' +
    '<ph-deadline-list id="deadline-list"></ph-deadline-list>' +
    '<ph-scan-readout id="scan-readout"></ph-scan-readout>' +
    '<ph-sensor-radar id="sensor-radar"></ph-sensor-radar>' +
    '<ph-sensor-panel id="sensor-panel"></ph-sensor-panel>' +
    '<ph-station-damage id="station-damage"></ph-station-damage>' +
    '<span id="footer-target"></span>' +
    '<span id="captain-auto-badge" hidden></span>',
  courier:
    '<ph-camera-select id="camera"></ph-camera-select>' +
    '<ph-red-alert id="red-alert"></ph-red-alert>' +
    '<ph-objective-list id="objectives"></ph-objective-list>' +
    '<div class="threat-readout" id="threat-row"><strong id="threat-bearing"></strong></div>' +
    '<ph-shield-facings id="shields"></ph-shield-facings>' +
    '<ph-power-controls id="power"></ph-power-controls>' +
    '<ph-battery-bar id="battery"></ph-battery-bar>' +
    '<ph-hull-integrity id="hull"></ph-hull-integrity>' +
    '<ph-repair-teams id="repair"></ph-repair-teams>' +
    '<ph-navigation-map id="nav"></ph-navigation-map>' +
    '<ph-comms-contact-list id="contacts"></ph-comms-contact-list>' +
    '<ph-comms-current-message id="message"></ph-comms-current-message>' +
    '<ph-station-damage id="damage"></ph-station-damage>',
};

// ── Battleship: the reference hull ───────────────────────────────────────────
describe('battleship captain renderStation', () => {
  beforeEach(() => mount(FIXTURES.battleship));

  const base = {
    camera_views: ['camera_fore', 'cinematic'], view_direction: 'camera_fore', viewscreen_auto: true,
    red_alert: true, weapons_hold: false, red_alert_auto: false,
    objectives: [{ id: 'o1', text: 'Hold the line', status: 'active' }], boosted_objective_id: 'o1',
    own_hull: { pct: 0.8 },
    blips: [{ uuid: 'e1' }],
    captain_auto: true,
  };

  it('drives camera, red-alert, objectives and station-damage from the flat payload', () => {
    battleshipRender(base, document);
    expect(el('camera-select').state).toEqual({ views: ['camera_fore', 'cinematic'], current_view: 'camera_fore', auto: true });
    expect(el('red-alert').state).toEqual({ active: true, hold: false, auto: false });
    expect(el('objective-list').state).toEqual({ objectives: base.objectives, boosted_objective_id: 'o1' });
    expect(el('station-damage').state).toEqual({ pct: 0.8 });
  });

  it('shows the plain contact-count footer with no color tint', () => {
    battleshipRender({ ...base, blips: [] }, document);
    expect(el('footer-target').textContent).toBe(t('console.common.no_target'));
    battleshipRender({ ...base, blips: [{ uuid: 'e1' }] }, document);
    expect(el('footer-target').textContent).toBe(t('console.common.contacts.one', { n: 1 }));
    expect(el('footer-target').style.color).toBe('');
  });

  it('reflects the AUTO badge', () => {
    battleshipRender(base, document);
    expect(el('captain-auto-badge').hidden).toBe(false);
    battleshipRender({ ...base, captain_auto: false }, document);
    expect(el('captain-auto-badge').hidden).toBe(true);
  });

  it('mirrors state into the hidden objectives/dir/alert Playwright test hooks', () => {
    battleshipRender(base, document);
    const rows = el('objectives').querySelectorAll('.objective-data');
    expect(rows.length).toBe(1);
    expect(rows[0].dataset.id).toBe('o1');
    expect(rows[0].dataset.text).toBe('Hold the line');
    expect(rows[0].dataset.status).toBe('active');
    expect(el('dir').dataset.direction).toBe('camera_fore');
    expect(el('alert').dataset.redAlert).toBe('true');
  });
});

// ── Cruiser: flat payload, no AUTO badge, tinted footer ───────────────────────
describe('cruiser captain renderStation', () => {
  beforeEach(() => mount(FIXTURES.cruiser));

  const base = {
    camera_views: ['camera_fore'], view_direction: 'camera_fore', viewscreen_auto: false,
    red_alert: false, weapons_hold: false, red_alert_auto: false,
    objectives: [], boosted_objective_id: null,
    own_hull: { pct: 1 },
    blips: [],
  };

  it('drives the shared core from the flat payload', () => {
    cruiserRender(base, document);
    expect(el('camera-select').state).toEqual({ views: ['camera_fore'], current_view: 'camera_fore', auto: false });
    expect(el('station-damage').state).toEqual({ pct: 1 });
  });

  it('tints the footer by contact count', () => {
    cruiserRender({ ...base, blips: [] }, document);
    expect(el('footer-target').textContent).toBe(t('console.common.no_target'));
    expect(el('footer-target').style.color).toBe('var(--ink-faint)');
    cruiserRender({ ...base, blips: [{ uuid: 'e1' }, { uuid: 'e2' }] }, document);
    expect(el('footer-target').textContent).toBe(t('console.common.contacts.other', { n: 2 }));
    expect(el('footer-target').style.color).toBe('var(--ink-dim)');
  });

  it('carries no AUTO badge', () => {
    expect(el('captain-auto-badge')).toBeNull();
  });
});

// ── Destroyer: keyed payload + sensors tail ──────────────────────────────────
describe('destroyer captain renderStation', () => {
  beforeEach(() => mount(FIXTURES.destroyer));

  const cap = { camera_views: ['camera_fore'], view_direction: 'camera_fore', viewscreen_auto: true, red_alert: false, weapons_hold: false, red_alert_auto: true, objectives: [{ id: 'o1' }], boosted_objective_id: null, deadlines: [{ id: 'd1' }] };
  const sensors = { blips: [{ uuid: 's1' }], scan: { text: 'hull breach' }, target_uuid: 's1', target_name: 'Raider', sensors_auto: true };
  const payload = {
    systems: { captain: cap, 'red-alert': cap, viewscreen: cap, sensors, 'sensor-radar': sensors },
    own_hull: { pct: 0.6 },
  };

  it('reads the captain view via systemView', () => {
    destroyerRender(payload, document);
    expect(el('camera-select').state).toEqual({ views: ['camera_fore'], current_view: 'camera_fore', auto: true });
    expect(el('red-alert').state).toEqual({ active: false, hold: false, auto: true });
    expect(el('objective-list').state).toEqual({ objectives: [{ id: 'o1' }], boosted_objective_id: null });
    expect(el('station-damage').state).toEqual({ pct: 0.6 });
  });

  it('drives the sensor radar/panel, deadline list and scan readout from the sensors view', () => {
    destroyerRender(payload, document);
    expect(el('sensor-radar').state).toEqual(sensors);
    expect(el('sensor-panel').state).toEqual(sensors);
    expect(el('deadline-list').state).toEqual({ deadlines: [{ id: 'd1' }] });
    expect(el('scan-readout').state).toEqual({ scan: { text: 'hull breach' }, target_uuid: 's1' });
  });

  it('shows the locked-target-name footer, not a contact count', () => {
    destroyerRender(payload, document);
    expect(el('footer-target').textContent).toBe('◉ Raider');
    expect(el('footer-target').style.color).toBe('var(--tactical)');
    const noLock = { ...payload, systems: { ...payload.systems, sensors: { ...sensors, target_name: null }, 'sensor-radar': { ...sensors, target_name: null } } };
    destroyerRender(noLock, document);
    expect(el('footer-target').textContent).toBe(t('console.common.no_target'));
    expect(el('footer-target').style.color).toBe('var(--ink-faint)');
  });

  it('conjuncts red_alert_auto, viewscreen_auto and sensors_auto for the AUTO badge', () => {
    destroyerRender(payload, document);
    expect(el('captain-auto-badge').hidden).toBe(false);
    const capNotAuto = { ...cap, viewscreen_auto: false };
    const notAllAuto = { ...payload, systems: { ...payload.systems, captain: capNotAuto, 'red-alert': capNotAuto, viewscreen: capNotAuto } };
    destroyerRender(notAllAuto, document);
    expect(el('captain-auto-badge').hidden).toBe(true);
  });
});

// ── Courier: keyed payload, mega-bespoke tail ────────────────────────────────
describe('courier captain renderStation', () => {
  beforeEach(() => mount(FIXTURES.courier));

  const command = { camera_views: ['camera_fore', 'camera_aft', 'cinematic'], view_direction: 'cinematic', viewscreen_auto: false, red_alert: true, weapons_hold: true, red_alert_auto: false, objectives: [{ id: 'o1' }], boosted_objective_id: 'o1' };
  const shields = { facings: [{ id: 'fore' }], focused_facing: 'fore', shields_auto: true, threat_bearing: 45.4 };
  const power = { consoles: [{ id: 'reactor' }], power_auto: true, battery_charge: 40, battery_max: 100, battery_online: true, charging: true };
  const repair = { overall_hull: { pct: 0.7, destroyed_pct: 0.1 }, teams: [{ id: 't1' }], repair_auto: false, dispatch_targets: [{ id: 'x' }], damaged_systems: [{ id: 'y' }] };
  const nav = { blips: [{ uuid: 'n1' }], regions: [{ id: 'r1' }], radar_range: 900, ship_x: 5, ship_z: 6, ship_heading: 45, waypoint: { name: 'Beacon' } };
  const comms = { contacts: [{ id: 'c1' }], messages: [{ id: 'm1', is_read: true }, { id: 'm2', is_read: false }], rejection: null };
  const payload = {
    systems: {
      captain: command, viewscreen: command, 'red-alert': command,
      'shields-system': shields, 'power-reactor': power, 'power-battery': power,
      repair, navigation: nav, comms,
    },
    own_hull: { pct: 0.4 },
  };

  it('filters the camera view list to fore + cinematic only', () => {
    courierRender(payload, document);
    expect(el('camera').state).toEqual({ views: ['camera_fore', 'cinematic'], current_view: 'cinematic', auto: false });
  });

  it('drives objectives under the courier-specific id and station-damage', () => {
    courierRender(payload, document);
    expect(el('objectives').state).toEqual({ objectives: [{ id: 'o1' }], boosted_objective_id: 'o1' });
    expect(el('damage').state).toEqual({ pct: 0.4 });
  });

  it('drives shields, the threat-bearing readout, power, battery, hull and repair from the absorbed systems', () => {
    courierRender(payload, document);
    expect(el('shields').state).toEqual({ facings: [{ id: 'fore' }], focused_facing: 'fore', auto: true });
    expect(el('threat-row').classList.contains('active')).toBe(true);
    expect(el('threat-bearing').textContent).toBe('45°M');
    expect(el('power').state).toEqual({ groups: [{ id: 'reactor' }], auto: true });
    expect(el('battery').state).toEqual({ level_pct: 40, charging: true, emergency_threshold_pct: 20 });
    expect(el('hull').state).toEqual({ total_pct: 0.7, destroyed_pct: 0.1 });
    expect(el('repair').state).toEqual({ teams: [{ id: 't1' }], auto: false, targets: [{ id: 'x' }], damaged: [{ id: 'y' }] });
  });

  it('hides the threat-bearing readout when Sensors holds no threat', () => {
    const noThreat = { ...payload, systems: { ...payload.systems, 'shields-system': { ...shields, threat_bearing: null } } };
    courierRender(noThreat, document);
    expect(el('threat-row').classList.contains('active')).toBe(false);
    expect(el('threat-bearing').textContent).toBe('—');
  });

  it('drives the Nav overlay map and Comms overlay thread from the absorbed systems', () => {
    courierRender(payload, document);
    expect(el('nav').state).toEqual({ blips: [{ uuid: 'n1' }], regions: [{ id: 'r1' }], range: 900, ship_pos: { x: 5, z: 6 }, ship_heading: 45, waypoint: { name: 'Beacon' } });
    expect(el('contacts').state).toEqual({ contacts: [{ id: 'c1' }] });
    expect(el('message').state).toEqual({ thread: { id: 'm2', is_read: false }, rejection: null });
  });

  it('carries no contact-count footer and no AUTO badge', () => {
    courierRender(payload, document);
    expect(el('footer-target')).toBeNull();
    expect(el('captain-auto-badge')).toBeNull();
  });
});
