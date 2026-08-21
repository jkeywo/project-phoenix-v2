// @vitest-environment jsdom
//
// tests/client/helm-nav-keyboard.test.js — the helm family and navigation
// composites' keyboard contract at the component level (issue #1176, the
// #1170 sweep). The keyboard-only Playwright smoke proves the principal
// actions fire end to end in a real browser; this fast suite pins the pure-DOM
// pieces: the role + accessible name + single Tab stop on every composite, and
// the handlers that dispatch a named action straight from a key —
//   • the two joysticks' flight bindings (the SAME set_helm / set_lateral_thrust
//     the pointer drag emits), and the DELIBERATE key-relay coexistence: one
//     document handler, keyed by a set, so a native + relayed press never
//     double-fires and Tab is never stolen;
//   • the navigation chart's arrow-cycled contact selection and Enter-to-commit
//     waypoint (the SAME set_navigation_waypoint the bar button sends);
//   • the boost button's Enter/Space hold-to-boost (the SAME set_boost the
//     pointer hold does), for a control whose pointer path is a hold, not a click.
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { t } from '../../gui/strings.js';
import { makeRadarCtx } from './radar-canvas-stub.js';
import '../../gui/components/ph-helm-joystick.js';
import '../../gui/components/ph-lateral-thrust-joystick.js';
import '../../gui/components/ph-navigation-map.js';
import '../../gui/components/ph-helm-radar.js';
import '../../gui/components/ph-boost-btn.js';

let rafCb = null;
let origGetContext;
let origRAF;
let origCARAF;
let origRO;
let origImage;

/** Run the most recently scheduled animation-frame callback. */
function tick() {
  if (rafCb) {
    const cb = rafCb;
    rafCb = null;
    cb(performance.now());
  }
}

beforeEach(() => {
  document.body.innerHTML = '';
  window.sendAction = vi.fn();
  rafCb = null;
  const fakeCtx = makeRadarCtx();
  origGetContext = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = function () { return fakeCtx; };
  origRAF = window.requestAnimationFrame;
  window.requestAnimationFrame = vi.fn((cb) => { rafCb = cb; return 1; });
  origCARAF = window.cancelAnimationFrame;
  window.cancelAnimationFrame = vi.fn();
  origRO = window.ResizeObserver;
  window.ResizeObserver = function () { return { observe: () => {}, disconnect: () => {} }; };
  origImage = window.Image;
  window.Image = class {
    constructor() { this.naturalWidth = 64; this.naturalHeight = 64; this.complete = true; }
  };
  Object.defineProperty(window, 'devicePixelRatio', { value: 2, configurable: true });
});

afterEach(() => {
  document.body.innerHTML = '';   // disconnects components → removes their listeners
  delete window.sendAction;
  HTMLCanvasElement.prototype.getContext = origGetContext;
  window.requestAnimationFrame = origRAF;
  window.cancelAnimationFrame = origCARAF;
  window.ResizeObserver = origRO;
  window.Image = origImage;
});

function mount(tag) {
  document.body.innerHTML = `<${tag} id="el"></${tag}>`;
  return document.getElementById('el');
}

/** A document-level keydown/keyup, as the flight bindings and key-relay deliver. */
function docKey(type, code, key) {
  const ev = new KeyboardEvent(type, { code, key: key || code, bubbles: true, cancelable: true });
  document.dispatchEvent(ev);
  return ev;
}

// ── Composite role + accessible name + Tab stop (AC #1) ──────────────────────

describe('composite role + accessible name (AC #1)', () => {
  it('the helm joystick is a named, focusable group', () => {
    const el = mount('ph-helm-joystick');
    expect(el.getAttribute('role')).toBe('group');
    expect(el.getAttribute('aria-label')).toBe(t('component.helm_joystick.label'));
    expect(el.getAttribute('tabindex')).toBe('0');
  });

  it('the lateral thruster is a named, focusable group', () => {
    const el = mount('ph-lateral-thrust-joystick');
    expect(el.getAttribute('role')).toBe('group');
    expect(el.getAttribute('aria-label')).toBe(t('component.lateral.label'));
    expect(el.getAttribute('tabindex')).toBe('0');
  });

  it('the navigation chart is a named, focusable group', () => {
    const el = mount('ph-navigation-map');
    expect(el.getAttribute('role')).toBe('group');
    expect(el.getAttribute('aria-label')).toBe(t('component.navigation_map.label'));
    expect(el.getAttribute('tabindex')).toBe('0');
  });

  it('the helm radar is a named group whose ON SCREEN button is the one control', () => {
    const el = mount('ph-helm-radar');
    expect(el.getAttribute('role')).toBe('group');
    expect(el.getAttribute('aria-label')).toBe(t('component.helm_radar.label'));
    // A passive display scope takes no tabindex of its own — an operationless
    // Tab stop would be a wrong stop; its native button carries the keyboard.
    expect(el.hasAttribute('tabindex')).toBe(false);
    const btn = el.shadowRoot.getElementById('on-screen-btn');
    expect(btn).toBeTruthy();
    expect(btn.tagName).toBe('BUTTON');
  });
});

// ── Helm joystick: keyboard flight + key-relay coexistence (AC #2) ───────────

describe('helm joystick keyboard flight (AC #2)', () => {
  it('an arrow key drives the SAME set_helm the pointer drag emits', () => {
    mount('ph-helm-joystick');
    docKey('keydown', 'ArrowUp');   // forward thrust
    tick();
    expect(window.sendAction).toHaveBeenCalledTimes(1);
    const [action, payload] = window.sendAction.mock.calls[0];
    expect(action).toBe('set_helm');
    expect(payload.thrust).toBeCloseTo(1, 5);
    expect(payload.yaw).toBe(0);
  });

  it('a native + relayed press of the same key fires set_helm ONCE (no double-fire)', () => {
    mount('ph-helm-joystick');
    // The real event and the gui/key-relay.js copy of it are both plain
    // document keydowns; the handler keys its state by code, so the pair
    // collapses to one held key and one send per frame.
    docKey('keydown', 'ArrowUp');
    docKey('keydown', 'ArrowUp');
    tick();
    expect(window.sendAction).toHaveBeenCalledTimes(1);
  });

  it('releasing the key snaps the stick back to zero', () => {
    mount('ph-helm-joystick');
    docKey('keydown', 'ArrowUp');
    tick();
    window.sendAction.mockClear();
    docKey('keyup', 'ArrowUp');
    tick();
    const last = window.sendAction.mock.calls.at(-1);
    expect(last[0]).toBe('set_helm');
    expect(last[1].thrust === 0 || Object.is(last[1].thrust, -0)).toBe(true);
    expect(last[1].yaw === 0 || Object.is(last[1].yaw, -0)).toBe(true);
  });

  it('does not steal Tab — a non-flight key is left for the browser', () => {
    mount('ph-helm-joystick');
    const ev = docKey('keydown', 'Tab');
    tick();
    expect(ev.defaultPrevented).toBe(false);
    expect(window.sendAction).not.toHaveBeenCalled();
  });
});

// ── Lateral thruster: keyboard strafe (AC #2) ───────────────────────────────

describe('lateral thruster keyboard strafe (AC #2)', () => {
  it('an arrow key drives the SAME set_lateral_thrust the pointer drag emits', () => {
    mount('ph-lateral-thrust-joystick');
    docKey('keydown', 'ArrowLeft');   // port
    tick();
    expect(window.sendAction).toHaveBeenCalledTimes(1);
    const [action, payload] = window.sendAction.mock.calls[0];
    expect(action).toBe('set_lateral_thrust');
    expect(payload.lateral).toBeCloseTo(-1, 5);
  });
});

// ── Navigation chart: arrow-cycle select + Enter commits waypoint (AC #2/#4) ─

describe('navigation chart keyboard operation (AC #2/#4)', () => {
  const CONTACTS = [
    { uuid: 'alpha', name: 'Alpha Station', kind: 'station', stance: 'friendly', world_x: 100, world_z: 0 },
    { uuid: 'bravo', name: 'Bravo Depot', kind: 'station', stance: 'neutral', world_x: -50, world_z: 80 },
  ];

  function chartWithContacts() {
    const el = mount('ph-navigation-map');
    el.state = { blips: CONTACTS, range: 5000, ship_pos: { x: 0, z: 0 }, ship_heading: 0, waypoint: null };
    return el;
  }

  it('an arrow key selects a contact — same overlay + navselect as a tap', () => {
    const el = chartWithContacts();
    const selected = [];
    el.addEventListener('navselect', (e) => selected.push(e.detail));
    el.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true, cancelable: true }));
    expect(el.shadowRoot.getElementById('ov-name').textContent).toBe('Alpha Station');
    expect(selected.at(-1)).toMatchObject({ uuid: 'alpha' });
    // The SET AS WAYPOINT bar button is now shown (there is a selection).
    expect(el.shadowRoot.getElementById('btn-set-selected').classList.contains('show')).toBe(true);
  });

  it('Enter commits the selected contact through the SAME action as SET AS WAYPOINT', () => {
    const el = chartWithContacts();
    el.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true, cancelable: true })); // → alpha
    el.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
    expect(window.sendAction).toHaveBeenCalledWith('set_navigation_waypoint',
      { x: 100, z: 0, source_uuid: 'alpha' });
  });

  it('arrows cycle through the contacts before Enter commits', () => {
    const el = chartWithContacts();
    el.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true, cancelable: true })); // → alpha
    el.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true, cancelable: true })); // → bravo
    el.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
    expect(window.sendAction).toHaveBeenLastCalledWith('set_navigation_waypoint',
      { x: -50, z: 80, source_uuid: 'bravo' });
  });

  it('Enter with nothing selected does not commit a waypoint', () => {
    const el = chartWithContacts();
    el.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
    expect(window.sendAction).not.toHaveBeenCalled();
  });

  it('Enter on the FOCUSED clear button clears — the host does not also SET (guard)', () => {
    // A waypoint set AND a contact selected: both SET AS WAYPOINT and CLEAR
    // WAYPOINT bar buttons show, so CLEAR can be the focused Tab stop.
    const el = mount('ph-navigation-map');
    el.state = { blips: CONTACTS, range: 5000, ship_pos: { x: 0, z: 0 }, ship_heading: 0, waypoint: { x: 10, z: 20 } };
    el.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true, cancelable: true })); // → alpha
    const clearBtn = el.shadowRoot.getElementById('btn-clear-waypoint');
    const setBtn = el.shadowRoot.getElementById('btn-set-selected');
    expect(clearBtn.classList.contains('show')).toBe(true);
    expect(setBtn.classList.contains('show')).toBe(true);
    window.sendAction.mockClear();

    // Enter on the focused CLEAR button composes+bubbles up to the host handler.
    const ev = new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, composed: true, cancelable: true });
    clearBtn.dispatchEvent(ev);
    // Guard: the host saw a descendant target, so it neither committed a
    // waypoint nor swallowed the key from the native button.
    expect(window.sendAction).not.toHaveBeenCalledWith('set_navigation_waypoint', expect.anything());
    expect(ev.defaultPrevented).toBe(false);

    // The button's own activation (the browser synthesises a click) clears.
    clearBtn.click();
    expect(window.sendAction).toHaveBeenCalledWith('clear_navigation_waypoint', {});
    expect(window.sendAction).not.toHaveBeenCalledWith('set_navigation_waypoint', expect.anything());
  });
});

// ── Boost: Enter/Space hold-to-boost (AC #2/#4) ─────────────────────────────

describe('boost hold-to-boost from the keyboard (AC #2/#4)', () => {
  it('Enter down engages and Enter up releases the SAME set_boost the pointer hold does', () => {
    const el = mount('ph-boost-btn');
    el.state = { available: true, active: false, recharge_pct: 100 };
    const btn = el.shadowRoot.getElementById('btn');
    btn.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, composed: true, cancelable: true }));
    btn.dispatchEvent(new KeyboardEvent('keyup', { key: 'Enter', bubbles: true, composed: true, cancelable: true }));
    expect(window.sendAction.mock.calls).toEqual([
      ['set_boost', { active: true }],
      ['set_boost', { active: false }],
    ]);
  });

  it('Space works the same way', () => {
    const el = mount('ph-boost-btn');
    el.state = { available: true, active: false, recharge_pct: 100 };
    const btn = el.shadowRoot.getElementById('btn');
    btn.dispatchEvent(new KeyboardEvent('keydown', { key: ' ', bubbles: true, composed: true, cancelable: true }));
    btn.dispatchEvent(new KeyboardEvent('keyup', { key: ' ', bubbles: true, composed: true, cancelable: true }));
    expect(window.sendAction.mock.calls.map((c) => c[1].active)).toEqual([true, false]);
  });

  it('a blur while the key is held releases boost once — no stuck-true', () => {
    // The keyup can be eaten by a focus loss; the button's blur handler must
    // release the held key so boost does not latch on with the battery
    // draining behind a hidden tab.
    const el = mount('ph-boost-btn');
    el.state = { available: true, active: false, recharge_pct: 100 };
    const btn = el.shadowRoot.getElementById('btn');
    btn.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, composed: true, cancelable: true }));
    btn.dispatchEvent(new FocusEvent('blur'));
    expect(window.sendAction.mock.calls).toEqual([
      ['set_boost', { active: true }],
      ['set_boost', { active: false }],
    ]);
  });
});
