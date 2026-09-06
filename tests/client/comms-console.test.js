// @vitest-environment jsdom
/**
 * tests/client/comms-console.test.js — the two Comms consoles on one
 * renderer (issue #1235, T4.C3 chunk 3 — final chunk of the console-seam
 * programme).
 *
 * Only the battleship and cruiser mount a dedicated Comms Station (the
 * destroyer and courier have none). Each hull's `.html` imports its
 * `renderStation` from `gui/<class>/comms.console.js`; this suite imports
 * the SAME functions and drives them against a jsdom fixture, so the
 * contact-list/hail-list/current-message contract is
 * asserted per hull without a browser.
 *
 * The custom-element modules are deliberately NOT imported: an un-upgraded
 * `<ph-*>` element is a plain `HTMLUnknownElement`, so assigning `.state`
 * stores a readable property and we assert the exact object the console
 * pushed, with no shadow-DOM machinery in the way.
 */
import { describe, it, expect, beforeEach } from 'vitest';
import { t } from '../../gui/strings.js';
import { renderStation as battleshipRender } from '../../gui/battleship/comms.console.js';
import { renderStation as rawCruiserRender } from '../../gui/cruiser/comms.console.js';
import { withConsoleFamilyProjection } from './console-family-fixture.js';

const cruiserRender = (payload, doc) => rawCruiserRender(withConsoleFamilyProjection(payload), doc);

function mount(markup) {
  document.body.innerHTML = markup;
}
const el = (id) => document.getElementById(id);

const FIXTURES = {
  battleship:
    '<ph-comms-contact-list id="comms-contact-list"></ph-comms-contact-list>' +
    '<ph-comms-hail-list id="comms-hail-list"></ph-comms-hail-list>' +
    '<ph-comms-current-message id="comms-current-message"></ph-comms-current-message>' +
    '<span id="footer-target"></span>' +
    '<span id="comms-auto-badge" hidden></span>',
  cruiser:
    '<div id="nav-view" hidden></div>' +
    '<div id="comms-view" hidden></div>' +
    '<ph-comms-contact-list id="comms-contact-list"></ph-comms-contact-list>' +
    '<ph-comms-hail-list id="comms-hail-list"></ph-comms-hail-list>' +
    '<ph-comms-current-message id="comms-current-message"></ph-comms-current-message>' +
    '<ph-navigation-map id="navigation-map"></ph-navigation-map>' +
    '<ph-civilian-traffic id="civilian-traffic"></ph-civilian-traffic>' +
    '<ph-objective-list id="objective-list"></ph-objective-list>' +
    '<span id="nav-contact-count"></span>' +
    '<span id="waypoint-name"></span>' +
    '<button id="btn-on-screen"></button>' +
    '<span id="navigation-auto-badge" hidden></span>' +
    '<span id="footer-target"></span>' +
    '<span id="footer-right"></span>' +
    '<span id="comms-auto-badge" hidden></span>',
};

// ── Battleship: the reference hull, flat `comms` family ───────────────────────
describe('battleship comms renderStation', () => {
  beforeEach(() => {
    battleshipRender.resetSelection();
    mount(FIXTURES.battleship);
  });

  const base = {
    contacts: [{ id: 'c1' }],
    messages: [{ id: 'm1', is_read: true, sender_name: 'Old' }, { id: 'm2', is_read: false, sender_name: 'Ops' }],
    rejection: null,
    own_hull: { pct: 0.9 },
    comms_auto: true,
  };

  it('drives the contact list, hail list and current message from the flat payload', () => {
    battleshipRender(base, document);
    expect(el('comms-contact-list').state).toEqual({ contacts: [{ id: 'c1' }] });
    expect(el('comms-hail-list').state).toEqual({ ...base, selected_message_id: null });
    expect(el('comms-current-message').state).toEqual({ thread: { id: 'm2', is_read: false, sender_name: 'Ops' }, messages: base.messages, rejection: null });
  });

  it('owns local message selection and repaints the list, thread and footer together', () => {
    battleshipRender(base, document);
    expect(battleshipRender.selectMessage(base, 'm1', document)).toBe(true);
    expect(el('comms-hail-list').state.selected_message_id).toBe('m1');
    expect(el('comms-current-message').state.thread.id).toBe('m1');
    expect(el('footer-target').textContent).toBe('Old');
    expect(battleshipRender.currentMessage(base).id).toBe('m1');
    expect(battleshipRender.selectMessage(base, 'missing', document)).toBe(false);
    expect(el('comms-current-message').state.thread.id).toBe('m1');
  });

  it('shows the active hail sender name, or the localized fallback for an unnamed hail', () => {
    battleshipRender(base, document);
    expect(el('footer-target').textContent).toBe('Ops');
    battleshipRender({ ...base, messages: [{ id: 'm1', is_read: false }] }, document);
    expect(el('footer-target').textContent).toBe(t('console.common.active_hail'));
  });

  it('selects a live Critical hail without making it modal', () => {
    const messages = [
      { id: 'm1', thread_id: 'routine', is_read: false, priority: 'Routine' },
      { id: 'm2', thread_id: 'lark', is_read: true, priority: 'Critical', selected_response: null },
    ];
    battleshipRender({ ...base, messages }, document);
    expect(el('comms-current-message').state).toEqual({
      thread: messages[1], messages, rejection: null,
    });
  });

  it('shows the no-active-hail fallback with no messages', () => {
    battleshipRender({ ...base, messages: [] }, document);
    expect(el('footer-target').textContent).toBe(t('console.common.no_active_hail'));
  });

  it('reflects the AUTO badge from comms_auto alone', () => {
    battleshipRender(base, document);
    expect(el('comms-auto-badge').hidden).toBe(false);
    battleshipRender({ ...base, comms_auto: false }, document);
    expect(el('comms-auto-badge').hidden).toBe(true);
  });

  it('carries no navigation map or footer-right message count', () => {
    expect(el('navigation-map')).toBeNull();
    expect(el('footer-right')).toBeNull();
  });
});

// ── Cruiser: keyed payload, Navigation absorbed into the same Station ────────
describe('cruiser comms renderStation', () => {
  beforeEach(() => {
    rawCruiserRender.resetSelection();
    mount(FIXTURES.cruiser);
  });

  const comms = {
    contacts: [{ id: 'c1' }],
    messages: [{ id: 'm1', is_read: false }],
    rejection: 'console.common.no_target',
    comms_auto: true,
  };
  const nav = {
    blips: [{ uuid: 'n1' }], regions: [{ id: 'r1' }], radar_range: 4000,
    ship_x: 1, ship_z: 2, ship_heading: 90, waypoint: { name: 'Gate' },
    navigation_auto: true,
  };
  const payload = { systems: { comms, navigation: nav }, own_hull: { pct: 0.5 } };

  it('reads the comms view via projected Console Family for the shared core', () => {
    cruiserRender(payload, document);
    expect(el('comms-contact-list').state).toEqual({ contacts: [{ id: 'c1' }] });
    expect(el('comms-hail-list').state).toEqual({ ...comms, selected_message_id: null });
    expect(el('comms-current-message').state).toEqual({ thread: { id: 'm1', is_read: false }, messages: comms.messages, rejection: 'console.common.no_target' });
  });

  it('drives the navigation map from the absorbed navigation system, through the shared Navigation renderer', () => {
    cruiserRender(payload, document);
    expect(el('navigation-map').state).toEqual({
      blips: [{ uuid: 'n1' }], regions: [{ id: 'r1' }], range: 4000,
      ship_pos: { x: 1, z: 2 }, ship_heading: 90, waypoint: { name: 'Gate' }, auto: true,
    });
    expect(el('waypoint-name').textContent).toBe('Gate');
    expect(el('civilian-traffic').state).toEqual({ civilians: [], auto: true });
  });

  it('shows the Navigation full-panel view and hides Comms when the navigation family resolves non-empty (issue #1379)', () => {
    cruiserRender(payload, document);
    expect(el('nav-view').hidden).toBe(false);
    expect(el('comms-view').hidden).toBe(true);
  });

  it('shows the Comms full-panel view and hides Navigation when this load carries no navigation family', () => {
    const commsOnly = { systems: { comms }, own_hull: { pct: 0.5 } };
    cruiserRender(commsOnly, document);
    expect(el('comms-view').hidden).toBe(false);
    expect(el('nav-view').hidden).toBe(true);
  });

  // Issue #1379 review finding 1: the auxiliary "navigation" Station owns
  // exactly one System, so `buildConsoleStateInner` takes the single-family
  // FLAT branch and this document's load carries `system_ids`/
  // `system_families` with NO `.systems` key at all — never the keyed
  // `{ systems: {...} }` shape every other case in this suite constructs.
  // `familyView` alone resolves `{}` for that shape; the renderer must fall
  // back to the payload itself once `system_families` says it IS navigation.
  it('shows the Navigation full-panel view with real map/waypoint data from a genuinely flat single-family payload', () => {
    const flatNav = {
      ...nav,
      own_hull: { pct: 0.5 },
      system_ids: ['navigation'],
      system_families: { navigation: 'navigation' },
    };
    cruiserRender(flatNav, document);
    expect(el('nav-view').hidden).toBe(false);
    expect(el('comms-view').hidden).toBe(true);
    expect(el('navigation-map').state).toEqual({
      blips: [{ uuid: 'n1' }], regions: [{ id: 'r1' }], range: 4000,
      ship_pos: { x: 1, z: 2 }, ship_heading: 90, waypoint: { name: 'Gate' }, auto: true,
    });
    expect(el('waypoint-name').textContent).toBe('Gate');
  });

  it('shows the waypoint name in the footer, not a hail', () => {
    cruiserRender(payload, document);
    expect(el('footer-target').textContent).toBe('Gate');
    const unnamed = { ...payload, systems: { ...payload.systems, navigation: { ...nav, waypoint: { name: null } } } };
    cruiserRender(unnamed, document);
    expect(el('footer-target').textContent).toBe(t('console.common.waypoint'));
    const none = { ...payload, systems: { ...payload.systems, navigation: { ...nav, waypoint: null } } };
    cruiserRender(none, document);
    expect(el('footer-target').textContent).toBe(t('console.common.no_waypoint'));
  });

  it('shows a pluralized message count in footer-right', () => {
    cruiserRender(payload, document);
    expect(el('footer-right').textContent).toBe(t('console.comms.messages.one', { n: 1 }));
    const twoMsgs = { ...payload, systems: { ...payload.systems, comms: { ...comms, messages: [{ id: 'm1' }, { id: 'm2' }] } } };
    cruiserRender(twoMsgs, document);
    expect(el('footer-right').textContent).toBe(t('console.comms.messages.other', { n: 2 }));
  });

  it('conjuncts comms_auto and navigation_auto for the AUTO badge', () => {
    cruiserRender(payload, document);
    expect(el('comms-auto-badge').hidden).toBe(false);
    const navNotAuto = { ...payload, systems: { ...payload.systems, navigation: { ...nav, navigation_auto: false } } };
    cruiserRender(navNotAuto, document);
    expect(el('comms-auto-badge').hidden).toBe(true);
  });
});
