import { describe, it, expect, vi, afterEach } from 'vitest';
import {
  HIDDEN_ELEMENTS,
  HIDEABLE_CLASS,
  hideableElementNames,
  plannedChanges,
  applyChanges,
  applyHiddenElements,
} from '../../gui/hideable-elements.js';

// ── Minimal DOM stubs (Node, no jsdom) ───────────────────────────────────────
// Mirror the fake-object style used elsewhere in tests/client: just enough of
// the Element / Document surface that applyHiddenElements touches
// (querySelectorAll, getAttribute, classList.{add,remove,contains}).

function makeEl(hideableName, { hidden = false } = {}) {
  const classes = new Set(hidden ? [HIDEABLE_CLASS] : []);
  return {
    _name: hideableName,
    getAttribute(attr) {
      return attr === 'data-hideable' ? hideableName : null;
    },
    classList: {
      add: (c) => classes.add(c),
      remove: (c) => classes.delete(c),
      contains: (c) => classes.has(c),
    },
    isHidden() {
      return classes.has(HIDEABLE_CLASS);
    },
  };
}

/** A fake root whose querySelectorAll('[data-hideable]') returns `els`. */
function makeRoot(els) {
  return { querySelectorAll: () => els };
}

// ── hideableElementNames ─────────────────────────────────────────────────────

describe('hideableElementNames', () => {
  it('returns the three Tactical Low elements (matches tactical.toml)', () => {
    expect(hideableElementNames('Tactical', 'Low').sort()).toEqual(
      ['phaser_mode_selector', 'target_lock_button', 'torpedo_tube_selector']);
  });

  it('Tactical Std hides nothing', () => {
    expect(hideableElementNames('Tactical', 'Std')).toEqual([]);
  });

  it('returns the one Power Low element (matches power.toml)', () => {
    expect(hideableElementNames('Power', 'Low')).toEqual(['power_overflow_controls']);
  });

  it('Power Std hides nothing', () => {
    expect(hideableElementNames('Power', 'Std')).toEqual([]);
  });

  it('returns [] for a console with no complexity config', () => {
    expect(hideableElementNames('Helm', 'Low')).toEqual([]);
    expect(hideableElementNames('Sensors', 'Low')).toEqual([]);
  });

  it('returns [] for an unknown preset name', () => {
    expect(hideableElementNames('Tactical', 'Nonexistent')).toEqual([]);
  });

  it('returns a fresh array (mutating the result does not corrupt the table)', () => {
    const a = hideableElementNames('Tactical', 'Low');
    a.push('injected');
    expect(hideableElementNames('Tactical', 'Low')).not.toContain('injected');
    expect(HIDDEN_ELEMENTS.Tactical.Low).not.toContain('injected');
  });
});

// ── plannedChanges ───────────────────────────────────────────────────────────

describe('plannedChanges', () => {
  const tacticalEls = ['phaser_mode_selector', 'torpedo_tube_selector', 'target_lock_button'];

  it('Tactical Low with all registered hides three, shows none', () => {
    const c = plannedChanges(new Set(), 'Tactical', 'Low', tacticalEls);
    expect(c.toHide.sort()).toEqual([...tacticalEls].sort());
    expect(c.toShow).toEqual([]);
    expect(c.unknown).toEqual([]);
  });

  it('Tactical Low reports unregistered elements as unknown', () => {
    const c = plannedChanges(new Set(), 'Tactical', 'Low', []);
    expect(c.unknown.sort()).toEqual([...tacticalEls].sort());
  });

  it('Tactical Std shows previously-hidden elements', () => {
    const hidden = new Set(tacticalEls);
    const c = plannedChanges(hidden, 'Tactical', 'Std', tacticalEls);
    expect(c.toHide).toEqual([]);
    expect(c.toShow.sort()).toEqual([...tacticalEls].sort());
    expect(c.unknown).toEqual([]);
  });

  it('Power Low hides power_overflow_controls', () => {
    const c = plannedChanges(new Set(), 'Power', 'Low', ['power_overflow_controls']);
    expect(c.toHide).toEqual(['power_overflow_controls']);
    expect(c.toShow).toEqual([]);
    expect(c.unknown).toEqual([]);
  });

  it('Power Std shows a previously-hidden overflow control', () => {
    const c = plannedChanges(new Set(['power_overflow_controls']), 'Power', 'Std',
      ['power_overflow_controls']);
    expect(c.toHide).toEqual([]);
    expect(c.toShow).toEqual(['power_overflow_controls']);
  });

  it('accepts arrays as well as Sets for currentHidden / registered', () => {
    const c = plannedChanges(['phaser_mode_selector'], 'Tactical', 'Std', tacticalEls);
    expect(c.toShow).toEqual(['phaser_mode_selector']);
  });
});

// ── applyChanges (in-place hidden-set update) ────────────────────────────────

describe('applyChanges', () => {
  it('adds toHide names and removes toShow names in place', () => {
    const hidden = new Set(['a']);
    const ret = applyChanges(hidden, { toHide: ['b', 'c'], toShow: ['a'] });
    expect(ret).toBe(hidden);
    expect([...hidden].sort()).toEqual(['b', 'c']);
  });
});

// ── applyHiddenElements (DOM toggling) ───────────────────────────────────────

describe('applyHiddenElements', () => {
  afterEach(() => vi.restoreAllMocks());

  it('hides the matching elements for Tactical Low', () => {
    const els = [
      makeEl('phaser_mode_selector'),
      makeEl('torpedo_tube_selector'),
      makeEl('target_lock_button'),
    ];
    applyHiddenElements(makeRoot(els), 'Tactical', 'Low');
    expect(els.every((e) => e.isHidden())).toBe(true);
  });

  it('shows previously-hidden elements when switching Low → Std', () => {
    const els = [
      makeEl('phaser_mode_selector', { hidden: true }),
      makeEl('torpedo_tube_selector', { hidden: true }),
      makeEl('target_lock_button', { hidden: true }),
    ];
    applyHiddenElements(makeRoot(els), 'Tactical', 'Std');
    expect(els.some((e) => e.isHidden())).toBe(false);
  });

  it('leaves Power overflow control visible under Std, hidden under Low', () => {
    const el = makeEl('power_overflow_controls');
    applyHiddenElements(makeRoot([el]), 'Power', 'Std');
    expect(el.isHidden()).toBe(false);
    applyHiddenElements(makeRoot([el]), 'Power', 'Low');
    expect(el.isHidden()).toBe(true);
  });

  it('warns about preset names with no matching data-hideable element', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    // No elements present, but Tactical Low names three.
    applyHiddenElements(makeRoot([]), 'Tactical', 'Low');
    expect(warn).toHaveBeenCalledTimes(3);
    expect(warn.mock.calls[0][0]).toContain('Tactical');
  });

  it('does not warn when every preset element is present', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const els = [
      makeEl('phaser_mode_selector'),
      makeEl('torpedo_tube_selector'),
      makeEl('target_lock_button'),
    ];
    applyHiddenElements(makeRoot(els), 'Tactical', 'Low');
    expect(warn).not.toHaveBeenCalled();
  });

  it('is idempotent: re-applying the same preset is a no-op', () => {
    const els = [makeEl('phaser_mode_selector'), makeEl('torpedo_tube_selector'),
      makeEl('target_lock_button')];
    const root = makeRoot(els);
    applyHiddenElements(root, 'Tactical', 'Low');
    const second = applyHiddenElements(root, 'Tactical', 'Low');
    // Everything is already hidden, so a re-apply schedules nothing to show
    // and the elements stay hidden.
    expect(second.toShow).toEqual([]);
    expect(els.every((e) => e.isHidden())).toBe(true);
  });

  it('returns empty changes for a root without querySelectorAll', () => {
    expect(applyHiddenElements(null, 'Tactical', 'Low'))
      .toEqual({ toHide: [], toShow: [], unknown: [] });
    expect(applyHiddenElements({}, 'Tactical', 'Low'))
      .toEqual({ toHide: [], toShow: [], unknown: [] });
  });
});
