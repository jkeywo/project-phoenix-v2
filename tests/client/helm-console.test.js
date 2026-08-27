// @vitest-environment jsdom
/**
 * tests/client/helm-console.test.js — the three Helm consoles on one
 * renderer (issue #1235, T4.C3 chunk 2).
 *
 * Each hull's `.html` imports its `renderStation` from
 * `gui/<class>/helm.console.js`; this suite imports the SAME functions and
 * drives them against a jsdom fixture, so the radar/joystick/footer contract
 * is asserted per hull without a browser.
 *
 * The custom-element modules are deliberately NOT imported: an un-upgraded
 * `<ph-*>` element is a plain `HTMLUnknownElement`, so assigning `.state`
 * stores a readable property and we assert the exact object the console
 * pushed, with no shadow-DOM machinery in the way.
 */
import { describe, it, expect, beforeEach } from 'vitest';
import { t } from '../../gui/strings.js';
import { renderStation as battleshipRender } from '../../gui/battleship/helm.console.js';
import { renderStation as cruiserRender } from '../../gui/cruiser/helm.console.js';
import { renderStation as destroyerRender } from '../../gui/destroyer/helm.console.js';

function mount(markup) {
  document.body.innerHTML = markup;
}
const el = (id) => document.getElementById(id);

const BATTLESHIP_FIXTURE =
  '<ph-helm-radar id="helm-radar"></ph-helm-radar>' +
  '<ph-helm-joystick id="helm-joystick"></ph-helm-joystick>' +
  '<ph-impulse-btn id="impulse-btn"></ph-impulse-btn>' +
  '<ph-boost-btn id="boost-btn"></ph-boost-btn>' +
  '<ph-station-damage id="station-damage"></ph-station-damage>' +
  '<span id="helm-auto-badge" hidden></span>' +
  '<span id="footer-target"></span>';

const CRUISER_FIXTURE =
  '<ph-helm-radar id="helm-radar"></ph-helm-radar>' +
  '<ph-helm-joystick id="helm-joystick"></ph-helm-joystick>' +
  '<ph-lateral-thrust-joystick id="lateral-thrust-joystick"></ph-lateral-thrust-joystick>' +
  '<ph-impulse-btn id="impulse-btn"></ph-impulse-btn>' +
  '<ph-boost-btn id="boost-btn"></ph-boost-btn>' +
  '<ph-station-damage id="station-damage"></ph-station-damage>' +
  '<span id="helm-auto-badge" hidden></span>' +
  '<span id="footer-target"></span>';

const DESTROYER_FIXTURE =
  '<ph-helm-radar id="helm-radar"></ph-helm-radar>' +
  '<ph-helm-joystick id="helm-joystick"></ph-helm-joystick>' +
  '<ph-lateral-thrust-joystick id="lateral-thrust-joystick"></ph-lateral-thrust-joystick>' +
  '<ph-impulse-btn id="impulse-btn"></ph-impulse-btn>' +
  '<ph-boost-btn id="boost-btn"></ph-boost-btn>' +
  '<ph-station-damage id="station-damage"></ph-station-damage>' +
  '<span id="helm-auto-badge" hidden></span>' +
  '<span id="footer-target">NO TARGET</span>' +
  '<div id="dock-panel" hidden>' +
    '<button id="dock-btn"></button><span id="dock-status"></span>' +
    '<div id="dock-refusal" hidden></div>' +
  '</div>' +
  '<div id="tow-load-panel" hidden><span id="tow-load-target"></span></div>';

const RADAR_PAYLOAD = { blips: [{ id: 'b1' }], range: 700, x: 10, z: 20, ship_heading: 90, speed: 5, on_screen: true, engine_port_thrust: 0.5, engine_stbd_thrust: 0.6, hostile_arcs: [{ id: 'h1' }], hostile_arc_color: '#f00' };

describe('battleship helm renderStation', () => {
  beforeEach(() => mount(BATTLESHIP_FIXTURE));

  const payload = { ...RADAR_PAYLOAD, helm_auto: true, impulse_charge_progress: 0.4, boost_enabled: true, boost_active: false, boost_battery: 0.8, own_hull: { pct: 0.9 } };

  it('drives the radar, joystick, impulse and boost from the flat payload', () => {
    battleshipRender(payload, document);
    expect(el('helm-radar').state).toEqual({
      blips: [{ id: 'b1' }], range: 700, x: 10, z: 20, ship_heading: 90, speed: 5, on_screen_active: true,
      config: {}, engine_port_thrust: 0.5, engine_stbd_thrust: 0.6, hostile_arcs: [{ id: 'h1' }], hostile_arc_color: '#f00',
    });
    expect(el('helm-joystick').state).toEqual({ auto: true });
    expect(el('impulse-btn').state).toEqual({ state: 'charging', charge_pct: 40, auto: true });
    expect(el('boost-btn').state).toEqual({ available: true, active: false, recharge_pct: 80, auto: true });
    expect(el('station-damage').state).toEqual({ pct: 0.9 });
    expect(el('helm-auto-badge').hidden).toBe(false);
  });

  it('carries no lateral joystick', () => {
    expect(el('lateral-thrust-joystick')).toBeNull();
  });

  it('shows the plain contact-count footer with no zero-contacts fallback', () => {
    battleshipRender({ ...payload, blips: [] }, document);
    expect(el('footer-target').textContent).toBe(t('console.common.contacts.other', { n: 0 }));
    battleshipRender({ ...payload, blips: [{ id: 'b1' }] }, document);
    expect(el('footer-target').textContent).toBe(t('console.common.contacts.one', { n: 1 }));
    battleshipRender({ ...payload, blips: [{ id: 'b1' }, { id: 'b2' }] }, document);
    expect(el('footer-target').textContent).toBe(t('console.common.contacts.other', { n: 2 }));
  });
});

describe('cruiser helm renderStation', () => {
  beforeEach(() => mount(CRUISER_FIXTURE));

  const payload = { ...RADAR_PAYLOAD, helm_auto: false, lateral_auto: true, impulse_charge_progress: 0, boost_enabled: false, boost_active: false, boost_battery: null, own_hull: { pct: 1 } };

  it('drives the lateral joystick alongside the shared core', () => {
    cruiserRender(payload, document);
    expect(el('lateral-thrust-joystick').state).toEqual({ auto: true });
    expect(el('impulse-btn').state).toEqual({ state: 'ready', charge_pct: 0, auto: false });
    expect(el('boost-btn').state).toEqual({ available: false, active: false, recharge_pct: 100, auto: false });
  });

  it('falls back to the localized no-target string at zero contacts and tints the footer', () => {
    cruiserRender({ ...payload, blips: [] }, document);
    expect(el('footer-target').textContent).toBe(t('console.common.no_target'));
    expect(el('footer-target').style.color).toBe('var(--ink-faint)');
  });

  it('prefixes the glyph and pluralizes at nonzero contacts, tinting the footer', () => {
    cruiserRender({ ...payload, blips: [{ id: 'b1' }] }, document);
    expect(el('footer-target').textContent).toBe('◉ ' + t('console.common.contacts.one', { n: 1 }));
    expect(el('footer-target').style.color).toBe('var(--ink-dim)');
    cruiserRender({ ...payload, blips: [{ id: 'b1' }, { id: 'b2' }] }, document);
    expect(el('footer-target').textContent).toBe('◉ ' + t('console.common.contacts.other', { n: 2 }));
  });

  it('carries no dock/tow-load panels', () => {
    expect(el('dock-panel')).toBeNull();
    expect(el('tow-load-panel')).toBeNull();
  });
});

describe('destroyer helm renderStation', () => {
  beforeEach(() => mount(DESTROYER_FIXTURE));

  const payload = { ...RADAR_PAYLOAD, helm_auto: true, lateral_auto: false, impulse_charge_progress: 0, boost_enabled: true, boost_active: true, boost_battery: 0.5, own_hull: { pct: 0.8 } };

  it('never touches the static footer target text', () => {
    destroyerRender(payload, document);
    expect(el('footer-target').textContent).toBe('NO TARGET');
  });

  it('hides the dock panel when no dock view is available/engaged/docked', () => {
    destroyerRender(payload, document);
    expect(el('dock-panel').hidden).toBe(true);
  });

  it('shows the dock panel and toggles dock/undock text+class', () => {
    const available = { ...payload, dock: { system_id: 'berthing-clamps', available: true, engaged: false, docked: false, available_target_name: 'console.tractor.idle' } };
    destroyerRender(available, document);
    expect(el('dock-panel').hidden).toBe(false);
    expect(el('dock-btn').classList.contains('docked')).toBe(false);
    expect(el('dock-btn').textContent).toBe(t('console.dock.dock'));
    expect(el('dock-btn').dataset.systemId).toBe('berthing-clamps');
    expect(el('dock-status').textContent).toContain(t('console.dock.available'));

    const docked = { ...payload, dock: { system_id: 'dock', available: false, engaged: true, docked: true, docked_to_name: 'console.tractor.idle' } };
    destroyerRender(docked, document);
    expect(el('dock-btn').classList.contains('docked')).toBe(true);
    expect(el('dock-btn').textContent).toBe(t('console.dock.undock'));
    expect(el('dock-btn').dataset.systemId).toBe('dock');
    expect(el('dock-status').textContent).toContain(t('console.dock.docked'));
  });

  it('shows a dock refusal when present and clears it when absent', () => {
    const refused = { ...payload, dock: { available: true, refusal: 'console.tractor.idle' } };
    destroyerRender(refused, document);
    expect(el('dock-refusal').hidden).toBe(false);
    const clear = { ...payload, dock: { available: true } };
    destroyerRender(clear, document);
    expect(el('dock-refusal').hidden).toBe(true);
  });

  it('shows the under-tow-load banner only while the tractor holds a target', () => {
    destroyerRender(payload, document);
    expect(el('tow-load-panel').hidden).toBe(true);
    const towed = { ...payload, tow_load: { active: true, target_name: 'console.tractor.idle' } };
    destroyerRender(towed, document);
    expect(el('tow-load-panel').hidden).toBe(false);
    expect(el('tow-load-target').textContent).toBe('· ' + t('console.tractor.idle'));
  });
});
