// @vitest-environment jsdom
//
// tests/client/combat-keyboard.test.js — the combat family's keyboard contract
// at the component level (issue #1177, the #1170 sweep for blasters/phasers/
// torpedoes' siblings: power and shield facings). The keyboard-only Playwright
// smoke proves the same actions fire end to end in a real browser; this fast
// suite pins the pieces that are pure DOM: the role + accessible name on each
// composite, the single tab stop roving leaves on the power steppers, the glyph
// steppers' names, the pips being presentational, and the shield-arc cursor —
// its arrow-cycle and the Enter that dispatches the SAME named action a tap does.
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { t } from '../../gui/strings.js';
import '../../gui/components/ph-power-controls.js';
import '../../gui/components/ph-shield-facings.js';

function mount(tag) {
  document.body.innerHTML = `<${tag} id="el"></${tag}>`;
  return document.getElementById('el');
}

beforeEach(() => {
  document.body.innerHTML = '';
  window.sendAction = vi.fn();
});

afterEach(() => {
  document.body.innerHTML = '';
  delete window.sendAction;
});

// ── ph-power-controls: named roving toolbar of glyph steppers ────────────────

describe('power controls: role, name and the single tab stop (AC #1)', () => {
  it('is a named vertical toolbar', () => {
    const el = mount('ph-power-controls');
    expect(el.getAttribute('role')).toBe('toolbar');
    expect(el.getAttribute('aria-orientation')).toBe('vertical');
    expect(el.getAttribute('aria-label')).toBe(t('component.power.title'));
  });

  it('the − / + glyph steppers carry accessible names, not just glyphs', () => {
    const el = mount('ph-power-controls');
    el.state = { groups: [{ id: 'weapons', label: 'Weapons', level: 2, min_level: 1, max_level: 4 }] };
    const decr = el.shadowRoot.querySelector('.mini-btn[data-action="decr"]');
    const incr = el.shadowRoot.querySelector('.mini-btn[data-action="incr"]');
    expect(decr.getAttribute('aria-label')).toBe(t('component.power.decrease'));
    expect(incr.getAttribute('aria-label')).toBe(t('component.power.increase'));
  });

  it('every group\'s steppers collapse to one Tab stop', () => {
    const el = mount('ph-power-controls');
    el.state = {
      groups: [
        { id: 'weapons', label: 'Weapons', level: 1, min_level: 1, max_level: 4 },
        { id: 'shields', label: 'Shields', level: 2, min_level: 1, max_level: 4 },
      ],
    };
    // 2 groups × (− +) = 4 steppers, exactly one tabbable.
    const steppers = Array.from(el.shadowRoot.querySelectorAll('.mini-btn'));
    expect(steppers.length).toBe(4);
    expect(steppers.filter((b) => b.tabIndex === 0).length).toBe(1);
  });

  it('the power pips are marked presentational, not unnamed controls', () => {
    const el = mount('ph-power-controls');
    el.state = { groups: [{ id: 'weapons', label: 'Weapons', level: 2, min_level: 1, max_level: 4 }] };
    const pips = Array.from(el.shadowRoot.querySelectorAll('.pip'));
    expect(pips.length).toBeGreaterThan(0);
    for (const pip of pips) expect(pip.getAttribute('role')).toBe('presentation');
  });

  it('activating a stepper dispatches set_power once (the same action a tap does)', () => {
    const el = mount('ph-power-controls');
    el.state = { groups: [{ id: 'weapons', label: 'Weapons', level: 1, min_level: 1, max_level: 4 }] };
    const incr = el.shadowRoot.querySelector('.mini-btn[data-action="incr"]');
    // A native button activates on Enter/Space in a browser (proven end-to-end
    // in the smoke); here the click its activation resolves to fires exactly the
    // one named action, with no keyboard-only fork to double it.
    incr.click();
    expect(window.sendAction).toHaveBeenCalledTimes(1);
    expect(window.sendAction).toHaveBeenCalledWith('set_power', { target: 'weapons', level: 2 });
  });
});

// ── ph-shield-facings: named focusable group, arrow-cycled arc cursor ────────

function shields(state) {
  const el = mount('ph-shield-facings');
  el.state = state;
  return el;
}

function keydown(el, key) {
  el.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true }));
}

describe('shield facings: role, name and keyboard reach (AC #1)', () => {
  it('is a named, focusable group', () => {
    const el = mount('ph-shield-facings');
    expect(el.getAttribute('role')).toBe('group');
    expect(el.getAttribute('aria-label')).toBe(t('component.shield_facings.title'));
    expect(el.getAttribute('tabindex')).toBe('0');
  });
});

describe('shield facings: arrow-cycled arc cursor commits the same action a tap does (AC #2/#3)', () => {
  it('an arrow moves the cursor and paints its ring; Enter focuses that facing', () => {
    const el = shields({
      facings: [{ arc_id: 'fore', label: 'Fore', hp: 100, max_hp: 100, online: true }],
      focused_facing: null,
    });
    keydown(el, 'ArrowRight');                    // cursor → first facing
    const cursor = el.shadowRoot.querySelector('.arc-path.kbd-cursor');
    expect(cursor).toBeTruthy();
    expect(cursor.getAttribute('data-facing-id')).toBe('fore');
    keydown(el, 'Enter');                          // commit
    expect(window.sendAction).toHaveBeenCalledWith('set_shield_focus', { arc_id: 'fore', focused: true });
  });

  it('cycles through facings before committing', () => {
    const el = shields({
      facings: [
        { arc_id: 'fore', label: 'Fore', hp: 100, max_hp: 100, online: true },
        { arc_id: 'port', label: 'Port', hp: 100, max_hp: 100, online: true },
      ],
      focused_facing: null,
    });
    keydown(el, 'ArrowDown');                      // → fore
    keydown(el, 'ArrowDown');                      // → port
    keydown(el, 'Enter');
    expect(window.sendAction).toHaveBeenLastCalledWith('set_shield_focus', { arc_id: 'port', focused: true });
  });

  it('the keyboard and the pointer run the SAME commit path (no fork, no double-fire)', () => {
    const state = {
      facings: [{ arc_id: 'fore', label: 'Fore', hp: 100, max_hp: 100, online: true }],
      focused_facing: null,
    };

    // Pointer: a tap on the hit-path.
    const pointerFn = vi.fn();
    window.sendAction = pointerFn;
    const a = shields(state);
    a.shadowRoot.querySelector('.hit-path').dispatchEvent(new MouseEvent('click'));

    // Keyboard: cursor then Enter.
    const keyFn = vi.fn();
    window.sendAction = keyFn;
    const b = shields(state);
    keydown(b, 'ArrowRight');
    keydown(b, 'Enter');

    expect(pointerFn).toHaveBeenCalledTimes(1);
    expect(keyFn).toHaveBeenCalledTimes(1);          // exactly one commit — no native+handler double
    expect(keyFn.mock.calls[0]).toEqual(pointerFn.mock.calls[0]);
  });

  it('toggles focus off when the cursor rests on the already-focused facing', () => {
    const el = shields({
      facings: [{ arc_id: 'fore', label: 'Fore', hp: 100, max_hp: 100, online: true }],
      focused_facing: 'fore',
    });
    keydown(el, 'ArrowRight');
    keydown(el, 'Enter');
    expect(window.sendAction).toHaveBeenCalledWith('set_shield_focus', { arc_id: 'fore', focused: false });
  });

  it('Enter with no cursor yet does nothing (no accidental focus change)', () => {
    const el = shields({
      facings: [{ arc_id: 'fore', label: 'Fore', hp: 100, max_hp: 100, online: true }],
      focused_facing: null,
    });
    keydown(el, 'Enter');
    expect(window.sendAction).not.toHaveBeenCalled();
  });

  it('in auto mode a keyboard commit sends nothing, mirroring the pointer (AUTO hint instead)', () => {
    const el = shields({
      facings: [{ arc_id: 'fore', label: 'Fore', hp: 100, max_hp: 100, online: true }],
      focused_facing: null,
      auto: true,
    });
    keydown(el, 'ArrowRight');
    keydown(el, 'Enter');
    expect(window.sendAction).not.toHaveBeenCalled();
  });

  it('drops the cursor if its facing leaves the ring', () => {
    const el = shields({
      facings: [{ arc_id: 'fore', label: 'Fore', hp: 100, max_hp: 100, online: true }],
      focused_facing: null,
    });
    keydown(el, 'ArrowRight');                      // cursor on 'fore'
    expect(el.shadowRoot.querySelector('.arc-path.kbd-cursor')).toBeTruthy();
    // A fresh push where 'fore' is gone must clear the stale ring.
    el.state = {
      facings: [{ arc_id: 'aft', label: 'Aft', hp: 100, max_hp: 100, online: true }],
      focused_facing: null,
    };
    expect(el.shadowRoot.querySelector('.arc-path.kbd-cursor')).toBeFalsy();
    keydown(el, 'Enter');                            // no cursor → no commit
    expect(window.sendAction).not.toHaveBeenCalled();
  });
});
