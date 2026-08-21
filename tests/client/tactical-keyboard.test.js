// @vitest-environment jsdom
//
// tests/client/tactical-keyboard.test.js — the Tactical console's keyboard
// contract at the component level (issue #1170). The keyboard-only Playwright
// smoke proves the same actions fire end to end in a real browser; this fast
// suite pins the pieces that are pure DOM: the role + accessible name on every
// composite, the single tab stop roving leaves, the glyph steppers' names, and
// the two handlers that dispatch a named action straight from a key —
// blasters' hold-to-fire and the radar's target cursor.
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { t } from '../../gui/strings.js';
import { makeRadarCtx } from './radar-canvas-stub.js';
import '../../gui/components/ph-phasers-controls.js';
import '../../gui/components/ph-blasters-controls.js';
import '../../gui/components/ph-torpedo-controls.js';
import '../../gui/components/ph-tactical-radar.js';

function mount(tag) {
  document.body.innerHTML = `<${tag} id="el"></${tag}>`;
  return document.getElementById('el');
}

function tabbable(host, selector) {
  return Array.from(host.shadowRoot.querySelectorAll(selector))
    .filter((el) => el.tabIndex === 0);
}

// ph-tactical-radar mounts a live ph-radar whose constructor observes itself
// and paints a frame straight away. jsdom supplies neither a ResizeObserver, a
// 2-D canvas context, an Image, nor a real rAF, so mounting the scope throws
// unless these are stubbed — the same harness every sibling radar test installs
// (see ph-tactical-radar.test.js / radar-canvas-stub.js).
let origGetContext;
let origRAF;
let origCARAF;
let origRO;
let origImage;

beforeEach(() => {
  document.body.innerHTML = '';
  window.sendAction = vi.fn();
  const fakeCtx = makeRadarCtx();
  origGetContext = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = function () { return fakeCtx; };
  origRAF = window.requestAnimationFrame;
  window.requestAnimationFrame = vi.fn(() => 1);
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
  document.body.innerHTML = '';
  delete window.sendAction;
  HTMLCanvasElement.prototype.getContext = origGetContext;
  window.requestAnimationFrame = origRAF;
  window.cancelAnimationFrame = origCARAF;
  window.ResizeObserver = origRO;
  window.Image = origImage;
});

describe('composite role + accessible name (AC #3)', () => {
  it('phasers is a named toolbar', () => {
    const el = mount('ph-phasers-controls');
    expect(el.getAttribute('role')).toBe('toolbar');
    expect(el.getAttribute('aria-label')).toBe(t('component.phasers.title'));
  });

  it('blasters is a named toolbar', () => {
    const el = mount('ph-blasters-controls');
    expect(el.getAttribute('role')).toBe('toolbar');
    expect(el.getAttribute('aria-label')).toBe(t('component.blasters.title'));
  });

  it('torpedoes is a named toolbar', () => {
    const el = mount('ph-torpedo-controls');
    expect(el.getAttribute('role')).toBe('toolbar');
    expect(el.getAttribute('aria-label')).toBe(t('component.torpedoes.title'));
  });

  it('the radar scope is a named, focusable group', () => {
    const el = mount('ph-tactical-radar');
    expect(el.getAttribute('role')).toBe('group');
    expect(el.getAttribute('aria-label')).toBe(t('component.tactical_radar.label'));
    expect(el.getAttribute('tabindex')).toBe('0');
  });
});

describe('roving tabindex leaves one Tab stop (AC #2)', () => {
  it('phasers: the mode toggle + fire buttons are one stop', () => {
    const el = mount('ph-phasers-controls');
    el.state = {
      mode: 'Manual',
      target_valid: true,
      banks: [
        { id: 'fore', label: 'Fore', fire_ready: true },
        { id: 'aft', label: 'Aft', fire_ready: true },
      ],
    };
    // mode toggle + 2 fire buttons = 3 controls, exactly one tabbable.
    expect(el.shadowRoot.querySelectorAll('#mode-toggle, #banks .btn').length).toBe(3);
    expect(tabbable(el, '#mode-toggle, #banks .btn').length).toBe(1);
  });

  it('torpedoes: every tube control collapses to one stop', () => {
    const el = mount('ph-torpedo-controls');
    el.state = {
      tubes: [
        { id: 'fore', label: 'Fore', loaded_count: 1, target_count: 1, volley_max: 2 },
        { id: 'aft', label: 'Aft', loaded_count: 0, target_count: 0, volley_max: 2 },
      ],
      magazine: { current: 4, max: 8 },
    };
    // 2 tubes × (− ＋ FIRE) = 6 buttons, exactly one tabbable.
    expect(el.shadowRoot.querySelectorAll('#tubes button').length).toBe(6);
    expect(tabbable(el, '#tubes button').length).toBe(1);
  });
});

describe('glyph steppers carry an accessible name (AC #3)', () => {
  it('the − / ＋ volley steppers are named, not just glyphs', () => {
    const el = mount('ph-torpedo-controls');
    el.state = { tubes: [{ id: 'fore', label: 'Fore', loaded_count: 0, target_count: 0, volley_max: 2 }] };
    const buttons = Array.from(el.shadowRoot.querySelectorAll('#tubes .mini-btn'));
    const labels = buttons.map((b) => b.getAttribute('aria-label'));
    expect(labels).toContain(t('component.torpedoes.volley_decrease'));
    expect(labels).toContain(t('component.torpedoes.volley_increase'));
  });
});

describe('blaster hold-to-fire from the keyboard (AC #2/#4)', () => {
  it('Enter down charges and Enter up fires the SAME named actions as the pointer', () => {
    const el = mount('ph-blasters-controls');
    el.state = { banks: [{ id: 'port', label: 'Port', fire_ready: true }] };
    const btn = el.shadowRoot.querySelector('#banks .btn');
    expect(btn).toBeTruthy();

    btn.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, composed: true, cancelable: true }));
    btn.dispatchEvent(new KeyboardEvent('keyup', { key: 'Enter', bubbles: true, composed: true, cancelable: true }));

    const actions = window.sendAction.mock.calls.map((c) => c[0]);
    expect(actions).toEqual(['charge_blaster_start', 'fire_blaster']);
    expect(window.sendAction.mock.calls[0][1]).toEqual({ bank: 'port' });
  });

  it('Space works the same way', () => {
    const el = mount('ph-blasters-controls');
    el.state = { banks: [{ id: 'stbd', label: 'Stbd', fire_ready: true }] };
    const btn = el.shadowRoot.querySelector('#banks .btn');
    btn.dispatchEvent(new KeyboardEvent('keydown', { key: ' ', bubbles: true, composed: true, cancelable: true }));
    btn.dispatchEvent(new KeyboardEvent('keyup', { key: ' ', bubbles: true, composed: true, cancelable: true }));
    expect(window.sendAction.mock.calls.map((c) => c[0])).toEqual(['charge_blaster_start', 'fire_blaster']);
  });

  it('a mid-charge blur releases the charge once, mirroring pointer mouseleave', () => {
    const el = mount('ph-blasters-controls');
    // The blur guard mirrors mouseleave: it fires only while the bank is charging.
    el.state = { banks: [{ id: 'port', label: 'Port', fire_ready: true, state: 'charging' }] };
    const btn = el.shadowRoot.querySelector('#banks .btn');

    // Hold Enter to charge, then move focus away before the keyup ever lands.
    btn.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, composed: true, cancelable: true }));
    btn.dispatchEvent(new FocusEvent('blur'));

    const actions = window.sendAction.mock.calls.map((c) => c[0]);
    expect(actions).toEqual(['charge_blaster_start', 'fire_blaster']); // released exactly once
    expect(window.sendAction.mock.calls[1][1]).toEqual({ bank: 'port' });
  });

  it('a blur with no charge in progress is a no-op (no stray fire)', () => {
    const el = mount('ph-blasters-controls');
    el.state = { banks: [{ id: 'port', label: 'Port', fire_ready: true }] }; // not charging
    const btn = el.shadowRoot.querySelector('#banks .btn');
    btn.dispatchEvent(new FocusEvent('blur'));
    expect(window.sendAction).not.toHaveBeenCalled();
  });
});

describe('radar target cursor from the keyboard (AC #2/#4)', () => {
  function radarWithContacts() {
    const el = mount('ph-tactical-radar');
    el.state = {
      blips: [
        { uuid: 'alpha', radar_x: 0.2, radar_y: 0.1, kind: 'ship' },
        { uuid: 'bravo', radar_x: -0.3, radar_y: -0.2, kind: 'ship' },
      ],
    };
    return el;
  }

  it('an arrow key moves the cursor and paints its ring; Enter locks that contact', () => {
    const el = radarWithContacts();
    el.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true, cancelable: true }));
    // A dashed cursor ring now exists in the overlay.
    expect(el.shadowRoot.querySelector('#keyboard-cursor circle')).toBeTruthy();
    el.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
    expect(window.sendAction).toHaveBeenCalledWith('set_target', { uuid: 'alpha' });
  });

  it('cycles through contacts before locking', () => {
    const el = radarWithContacts();
    el.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true, cancelable: true })); // → alpha
    el.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true, cancelable: true })); // → bravo
    el.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
    expect(window.sendAction).toHaveBeenLastCalledWith('set_target', { uuid: 'bravo' });
  });

  it('Enter with no cursor yet does nothing (no accidental lock)', () => {
    const el = radarWithContacts();
    el.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
    expect(window.sendAction).not.toHaveBeenCalled();
  });
});
