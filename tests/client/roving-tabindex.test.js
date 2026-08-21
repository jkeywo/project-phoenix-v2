// @vitest-environment jsdom
//
// tests/client/roving-tabindex.test.js — the shared roving-tabindex rule
// (issue #1170). `rovingKeyTarget` is the pure core every composite in the
// keyboard sweep reuses, so it is tested on its own before any DOM; the two
// DOM helpers are checked against a small hand-built group.
import { describe, it, expect } from 'vitest';
import {
  rovingKeyTarget, syncRovingTabindex, installRovingTabindex,
} from '../../gui/roving-tabindex.js';

describe('rovingKeyTarget', () => {
  it('wraps forward and backward like the Hero Bar ring', () => {
    expect(rovingKeyTarget(3, 0, 'ArrowDown')).toBe(1);
    expect(rovingKeyTarget(3, 2, 'ArrowDown')).toBe(0); // past the end → first
    expect(rovingKeyTarget(3, 0, 'ArrowUp')).toBe(2);   // before the start → last
    expect(rovingKeyTarget(3, 1, 'ArrowUp')).toBe(0);
  });

  it('Home and End jump to the ends regardless of orientation', () => {
    expect(rovingKeyTarget(5, 3, 'Home')).toBe(0);
    expect(rovingKeyTarget(5, 3, 'End')).toBe(4);
  });

  it('answers only the keys its orientation owns', () => {
    // A vertical toolbar consumes Up/Down and ignores Left/Right.
    expect(rovingKeyTarget(3, 0, 'ArrowDown', 'vertical')).toBe(1);
    expect(rovingKeyTarget(3, 0, 'ArrowRight', 'vertical')).toBe(-1);
    // A horizontal one is the mirror image.
    expect(rovingKeyTarget(3, 0, 'ArrowRight', 'horizontal')).toBe(1);
    expect(rovingKeyTarget(3, 0, 'ArrowDown', 'horizontal')).toBe(-1);
    // 'both' (a 2-D scope) consumes all four arrows.
    expect(rovingKeyTarget(3, 0, 'ArrowRight', 'both')).toBe(1);
    expect(rovingKeyTarget(3, 0, 'ArrowDown', 'both')).toBe(1);
  });

  it('returns -1 for a key it does not consume, so the handler can pass it on', () => {
    expect(rovingKeyTarget(3, 0, 'Enter')).toBe(-1);
    expect(rovingKeyTarget(3, 0, ' ')).toBe(-1);
    expect(rovingKeyTarget(3, 0, 'a')).toBe(-1);
  });

  it('is safe on an empty or single-item group', () => {
    expect(rovingKeyTarget(0, 0, 'ArrowDown')).toBe(-1);
    expect(rovingKeyTarget(1, 0, 'ArrowDown')).toBe(0);
    expect(rovingKeyTarget(1, 0, 'ArrowUp')).toBe(0);
  });

  it('treats an out-of-range current index as the first item', () => {
    expect(rovingKeyTarget(3, -1, 'ArrowDown')).toBe(1);
    expect(rovingKeyTarget(3, 9, 'ArrowDown')).toBe(1);
  });
});

function group(n) {
  const host = document.createElement('div');
  const items = [];
  for (let i = 0; i < n; i += 1) {
    const b = document.createElement('button');
    b.textContent = String(i);
    host.appendChild(b);
    items.push(b);
  }
  document.body.appendChild(host);
  return { host, items };
}

describe('syncRovingTabindex', () => {
  it('leaves exactly one item in the tab order', () => {
    const { items } = group(3);
    syncRovingTabindex(items, 1);
    expect(items.map((el) => el.tabIndex)).toEqual([-1, 0, -1]);
  });

  it('defaults to the item already tabbable, else the first', () => {
    const { items } = group(3);
    // A synced group has exactly one item at 0; prove the default preserves it.
    items[0].tabIndex = -1;
    items[1].tabIndex = -1;
    items[2].tabIndex = 0;
    expect(syncRovingTabindex(items)).toBe(2);
    expect(items.map((el) => el.tabIndex)).toEqual([-1, -1, 0]);
  });

  it('collapses a group with several tab stops down to the first', () => {
    // The un-synced initial state — every native button defaults to tabIndex 0.
    const { items } = group(3);
    expect(items.map((el) => el.tabIndex)).toEqual([0, 0, 0]);
    expect(syncRovingTabindex(items)).toBe(0);
    expect(items.map((el) => el.tabIndex)).toEqual([0, -1, -1]);
  });

  it('never parks the stop on a disabled item — falls back to the first enabled one', () => {
    const { items } = group(3);
    items[1].disabled = true;
    // Asking for the disabled index is refused; the stop settles on an enabled one.
    expect(syncRovingTabindex(items, 1)).toBe(0);
    expect(items.map((el) => el.tabIndex)).toEqual([0, -1, -1]);
  });

  it('is a no-op returning -1 on an empty group', () => {
    expect(syncRovingTabindex([])).toBe(-1);
  });
});

describe('installRovingTabindex', () => {
  it('moves focus and the single tab stop on an arrow key', () => {
    const { host, items } = group(3);
    syncRovingTabindex(items, 0);
    installRovingTabindex(host, { getItems: () => items, orientation: 'vertical' });
    items[0].focus();

    items[0].dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true, composed: true }));
    expect(document.activeElement).toBe(items[1]);
    expect(items.map((el) => el.tabIndex)).toEqual([-1, 0, -1]);
  });

  it('arrow-stepping skips a disabled control and never parks the stop on it', () => {
    const { host, items } = group(3);
    items[1].disabled = true; // an unfocusable middle control
    syncRovingTabindex(items, 0);
    installRovingTabindex(host, { getItems: () => items, orientation: 'vertical' });
    items[0].focus();

    // ArrowDown from the first enabled control jumps over the disabled one.
    items[0].dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true, composed: true }));
    expect(document.activeElement).toBe(items[2]);
    // The disabled control is left at -1 — the stop moved past it, not onto it.
    expect(items.map((el) => el.tabIndex)).toEqual([-1, -1, 0]);
  });

  it('leaves Enter and Space to the native button (no fork)', () => {
    const { host, items } = group(3);
    syncRovingTabindex(items, 1);
    installRovingTabindex(host, { getItems: () => items, orientation: 'vertical' });
    items[1].focus();

    const enter = new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, composed: true, cancelable: true });
    items[1].dispatchEvent(enter);
    expect(enter.defaultPrevented).toBe(false);      // not consumed
    expect(document.activeElement).toBe(items[1]);    // focus unmoved
    expect(items.map((el) => el.tabIndex)).toEqual([-1, 0, -1]);
  });

  it('uninstalls cleanly', () => {
    const { host, items } = group(2);
    syncRovingTabindex(items, 0);
    const uninstall = installRovingTabindex(host, { getItems: () => items });
    uninstall();
    items[0].focus();
    items[0].dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true, composed: true }));
    expect(document.activeElement).toBe(items[0]); // listener gone → no move
  });
});
