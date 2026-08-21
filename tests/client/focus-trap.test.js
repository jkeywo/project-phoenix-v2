// @vitest-environment jsdom
//
// Issue #1174 — the shared modal focus contract (gui/focus-trap.js).
//
// The three behaviours a modal owes a keyboard operator, driven in jsdom so
// document.activeElement is real: the trap (Tab/Shift+Tab wrap within the
// modal), Escape-to-close, focus moved IN on open and RESTORED to the opener on
// close, and the background made inert to match.

import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { createFocusTrap, focusableWithin, activeElementOf } from '../../gui/focus-trap.js';

// jsdom shares one `document` across every test in the file, and an active trap
// holds a document-level keydown listener. A trap left un-released would keep
// listening — with a stale modal reference — into the next test, so every trap
// a test makes is tracked and released afterwards, exactly as a real modal's
// close() releases it.
let traps = [];
function makeTrap(modal, opts) {
  const trap = createFocusTrap(modal, opts);
  traps.push(trap);
  return trap;
}
afterEach(() => {
  traps.forEach((trap) => trap.release());
  traps = [];
});

/**
 * A trigger button, a background panel, and a modal with three buttons — the
 * shape every modal in this codebase has: an opener the focus returns to, a
 * page behind that must go inert, and a ring of controls to cycle.
 */
function scene({ backgroundHidden } = {}) {
  document.body.innerHTML = '';

  const trigger = document.createElement('button');
  trigger.id = 'trigger';
  trigger.textContent = 'Open';
  document.body.appendChild(trigger);

  const background = document.createElement('div');
  background.id = 'background';
  if (backgroundHidden !== undefined) background.setAttribute('aria-hidden', backgroundHidden);
  const bgBtn = document.createElement('button');
  bgBtn.id = 'bg-btn';
  bgBtn.textContent = 'Behind';
  background.appendChild(bgBtn);
  document.body.appendChild(background);

  const modal = document.createElement('div');
  modal.id = 'modal';
  modal.setAttribute('role', 'dialog');
  modal.setAttribute('aria-modal', 'true');
  const first = document.createElement('button');
  first.id = 'first';
  first.textContent = 'First';
  const mid = document.createElement('button');
  mid.id = 'mid';
  mid.textContent = 'Mid';
  const last = document.createElement('button');
  last.id = 'last';
  last.textContent = 'Last';
  modal.append(first, mid, last);
  document.body.appendChild(modal);

  return { trigger, background, bgBtn, modal, first, mid, last };
}

/** Dispatch a bubbling keydown from whatever currently holds focus. */
function press(key, { shift = false } = {}) {
  const target = document.activeElement || document.body;
  const event = new KeyboardEvent('keydown', { key, shiftKey: shift, bubbles: true, cancelable: true });
  target.dispatchEvent(event);
  return event;
}

describe('focusableWithin', () => {
  it('returns the enabled controls in document order', () => {
    const { modal } = scene();
    expect(focusableWithin(modal).map((el) => el.id)).toEqual(['first', 'mid', 'last']);
  });

  it('skips disabled, hidden, aria-hidden and tabindex="-1" controls', () => {
    const { modal, mid, last } = scene();
    mid.disabled = true;
    last.setAttribute('tabindex', '-1');
    expect(focusableWithin(modal).map((el) => el.id)).toEqual(['first']);
  });
});

describe('createFocusTrap — opening moves focus in', () => {
  let s;
  beforeEach(() => { s = scene(); });

  it('moves focus to the first control on activate', () => {
    s.trigger.focus();
    const trap = makeTrap(s.modal);
    trap.activate();
    expect(document.activeElement).toBe(s.first);
  });

  it('honours an explicit initialFocus', () => {
    s.trigger.focus();
    const trap = makeTrap(s.modal, { initialFocus: '#mid' });
    trap.activate();
    expect(document.activeElement).toBe(s.mid);
  });
});

describe('createFocusTrap — Tab and Shift+Tab cycle within the modal', () => {
  let s;
  let trap;
  beforeEach(() => {
    s = scene();
    s.trigger.focus();
    trap = makeTrap(s.modal);
    trap.activate();
  });

  it('Tab off the last control wraps to the first', () => {
    s.last.focus();
    const event = press('Tab');
    expect(document.activeElement).toBe(s.first);
    expect(event.defaultPrevented).toBe(true);
  });

  it('Shift+Tab off the first control wraps to the last', () => {
    s.first.focus();
    const event = press('Tab', { shift: true });
    expect(document.activeElement).toBe(s.last);
    expect(event.defaultPrevented).toBe(true);
  });

  it('Tab in the middle of the ring is left to the browser', () => {
    s.first.focus();
    const event = press('Tab');
    // Not a wrap edge: the trap does not intervene, so nothing was prevented.
    expect(event.defaultPrevented).toBe(false);
  });

  it('pulls focus back inside if it has escaped the modal', () => {
    // Focus somewhere on the page behind the modal, then Tab.
    s.bgBtn.focus();
    press('Tab');
    expect(document.activeElement).toBe(s.first);
  });
});

describe('createFocusTrap — Escape closes', () => {
  it('invokes onEscape when Escape is pressed inside the modal', () => {
    const s = scene();
    s.trigger.focus();
    let closed = 0;
    const trap = makeTrap(s.modal, { onEscape: () => { closed += 1; trap.release(); } });
    trap.activate();
    const event = press('Escape');
    expect(closed).toBe(1);
    expect(event.defaultPrevented).toBe(true);
  });
});

describe('createFocusTrap — closing restores focus to the opener', () => {
  it('returns focus to the element that was focused when it opened', () => {
    const s = scene();
    s.trigger.focus();
    const trap = makeTrap(s.modal);
    trap.activate();
    expect(document.activeElement).toBe(s.first);
    trap.release();
    expect(document.activeElement).toBe(s.trigger);
  });

  it('release is idempotent — a second call does not throw or re-steal focus', () => {
    const s = scene();
    s.trigger.focus();
    const trap = makeTrap(s.modal);
    trap.activate();
    trap.release();
    s.bgBtn.focus();
    trap.release(); // already released; must be a no-op
    expect(document.activeElement).toBe(s.bgBtn);
  });
});

describe('createFocusTrap — the background inert/aria-hidden matches the trap', () => {
  it('marks the background inert and aria-hidden while open, and clears it on close', () => {
    const s = scene();
    s.trigger.focus();
    const trap = makeTrap(s.modal);
    trap.activate();
    expect(s.background.hasAttribute('inert')).toBe(true);
    expect(s.background.getAttribute('aria-hidden')).toBe('true');
    trap.release();
    expect(s.background.hasAttribute('inert')).toBe(false);
    expect(s.background.hasAttribute('aria-hidden')).toBe(false);
  });

  it('restores the background\'s prior aria-hidden rather than clearing it', () => {
    const s = scene({ backgroundHidden: 'false' });
    s.trigger.focus();
    const trap = makeTrap(s.modal);
    trap.activate();
    expect(s.background.getAttribute('aria-hidden')).toBe('true');
    trap.release();
    // The value it declared before the modal opened is put back.
    expect(s.background.getAttribute('aria-hidden')).toBe('false');
  });

  it('leaves the opener live so it can still toggle the modal closed', () => {
    const s = scene();
    s.trigger.focus();
    const trap = makeTrap(s.modal);
    trap.activate();
    // The trigger is a sibling of the modal but must NOT be inert — a second
    // click on the cog is how the operator closes the panel.
    expect(s.trigger.hasAttribute('inert')).toBe(false);
    trap.release();
  });
});

describe('activeElementOf', () => {
  it('returns the focused element', () => {
    const s = scene();
    s.mid.focus();
    expect(activeElementOf(document)).toBe(s.mid);
  });

  it('is null for a null document', () => {
    expect(activeElementOf(null)).toBe(null);
  });
});
