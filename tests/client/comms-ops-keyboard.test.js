// @vitest-environment jsdom
//
// tests/client/comms-ops-keyboard.test.js — the comms/ops/lobby list composites'
// keyboard contract at the component level (issue #1178, the #1170 sweep).
//
// The keyboard-only Playwright smoke proves the principal actions fire end to
// end in a real browser; this fast suite pins the pure-DOM pieces the sweep
// converted — the hail list, the objective list and the lobby ship picker,
// each of which was a stack of clickable <div>s the keyboard could not land on:
//
//   • role + accessible name on each host (a listbox, named from the same
//     string its console heading already shows);
//   • native <button role="option"> rows/cards — one Tab stop for the whole
//     list, arrows roving between the options, a single option tabbable;
//   • aria-selected tracking the real selection (the boosted objective, the
//     pending hull, the opened hail);
//   • a pointer click dispatching the named action — and the roving host
//     keydown NOT forking that action onto Enter (the #1176 composed-keydown
//     pitfall), so a browser's native Enter/Space activation is the only path.
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { t } from '../../gui/strings.js';
import '../../gui/components/ph-comms-hail-list.js';
import '../../gui/components/ph-objective-list.js';
import { setShipCards } from '../../gui/components/ph-ship-picker.js';
import '../../gui/components/ph-ship-picker.js';

beforeEach(() => {
  document.body.innerHTML = '';
  window.sendAction = vi.fn();
  setShipCards({});   // no card-art fetch in jsdom; render is complete without it
});

afterEach(() => {
  document.body.innerHTML = '';   // disconnects components → removes listeners
  delete window.sendAction;
  setShipCards(null);
});

/** Mount a tag, set its state, return the host element. */
function mount(tag, state) {
  document.body.innerHTML = `<${tag} id="el"></${tag}>`;
  const el = document.getElementById('el');
  el.state = state;
  return el;
}

/** The option rows/cards a component renders, in document order. */
function options(el, sel = '.row') {
  return Array.from(el.shadowRoot.querySelectorAll(sel));
}

/** A composed, bubbling keydown — as a real keypress crosses the shadow root. */
function keydown(target, key) {
  const ev = new KeyboardEvent('keydown', { key, bubbles: true, composed: true, cancelable: true });
  target.dispatchEvent(ev);
  return ev;
}

/** Exactly one member of `items` is in the tab order (tabindex 0). */
function singleTabStop(items) {
  return items.filter((i) => i.tabIndex === 0).length === 1
    && items.every((i) => i.tabIndex === 0 || i.tabIndex === -1);
}

// ── ph-comms-hail-list ───────────────────────────────────────────────────────

describe('ph-comms-hail-list (comms) keyboard contract', () => {
  const state = {
    messages: [
      { id: 'm1', sender_name: 'ALPHA', body: 'first hail', is_read: false },
      { id: 'm2', sender_name: 'BRAVO', body: 'second hail', is_read: true },
    ],
  };

  it('is a named listbox host (AC: role + name)', () => {
    const el = mount('ph-comms-hail-list', state);
    expect(el.getAttribute('role')).toBe('listbox');
    expect(el.getAttribute('aria-label')).toBe(t('component.comms_hails.label'));
  });

  it('renders native <button role="option"> rows named by their own text', () => {
    const el = mount('ph-comms-hail-list', state);
    const rows = options(el);
    expect(rows).toHaveLength(2);
    for (const r of rows) {
      expect(r.tagName).toBe('BUTTON');
      expect(r.getAttribute('role')).toBe('option');
    }
    expect(rows[0].textContent).toContain('ALPHA');
    expect(rows[0].textContent).toContain('first hail');
  });

  it('keeps exactly one Tab stop and roves it with the arrow keys', () => {
    const el = mount('ph-comms-hail-list', state);
    const rows = options(el);
    expect(singleTabStop(rows)).toBe(true);

    rows[0].focus();
    const ev = keydown(rows[0], 'ArrowDown');
    expect(ev.defaultPrevented).toBe(true);
    expect(el.shadowRoot.activeElement).toBe(rows[1]);
    expect(rows[1].tabIndex).toBe(0);
    expect(rows[0].tabIndex).toBe(-1);
  });

  it('a click dispatches select_comms_message and marks the row selected', () => {
    const el = mount('ph-comms-hail-list', state);
    const rows = options(el);
    rows[1].click();
    expect(window.sendAction).toHaveBeenCalledTimes(1);
    expect(window.sendAction).toHaveBeenCalledWith('select_comms_message', { message_id: 'm2' });
    expect(rows[1].getAttribute('aria-selected')).toBe('true');
    expect(rows[0].getAttribute('aria-selected')).toBe('false');
  });

  it('the roving host keydown does not fork the action onto Enter or Space', () => {
    const el = mount('ph-comms-hail-list', state);
    const rows = options(el);
    rows[0].focus();
    keydown(rows[0], 'Enter');
    keydown(rows[0], ' ');
    // Native <button> activation (browser-only) is the sole path; the host
    // handler must not also send. The smoke proves the browser Enter fires it.
    expect(window.sendAction).not.toHaveBeenCalled();
  });
});

// ── ph-objective-list (ops) ──────────────────────────────────────────────────

describe('ph-objective-list (ops) keyboard contract', () => {
  const state = {
    objectives: [
      { id: 'o1', text: 'Reach the beacon', done: false },
      { id: 'o2', text: 'Repair the hull', done: false },
    ],
    boosted_objective_id: 'o2',
  };

  it('is a named listbox host (AC: role + name)', () => {
    const el = mount('ph-objective-list', state);
    expect(el.getAttribute('role')).toBe('listbox');
    expect(el.getAttribute('aria-label')).toBe(t('component.objectives.label'));
  });

  it('renders native <button role="option"> rows and marks the boosted one selected', () => {
    const el = mount('ph-objective-list', state);
    const rows = options(el);
    expect(rows).toHaveLength(2);
    expect(rows[0].tagName).toBe('BUTTON');
    expect(rows[0].getAttribute('role')).toBe('option');
    expect(rows[0].textContent).toContain('Reach the beacon');
    expect(rows[0].getAttribute('aria-selected')).toBe('false');
    expect(rows[1].getAttribute('aria-selected')).toBe('true');   // boosted
    expect(singleTabStop(rows)).toBe(true);
  });

  it('roves the single Tab stop with the arrow keys', () => {
    const el = mount('ph-objective-list', state);
    const rows = options(el);
    rows[0].focus();
    keydown(rows[0], 'ArrowDown');
    expect(el.shadowRoot.activeElement).toBe(rows[1]);
    expect(rows[1].tabIndex).toBe(0);
  });

  it('a click dispatches set_objective_priority; the roving keydown does not fork it', () => {
    const el = mount('ph-objective-list', state);
    const rows = options(el);
    rows[0].click();
    expect(window.sendAction).toHaveBeenCalledTimes(1);
    expect(window.sendAction).toHaveBeenCalledWith('set_objective_priority', { id: 'o1' });

    window.sendAction.mockClear();
    rows[0].focus();
    keydown(rows[0], 'Enter');
    keydown(rows[0], ' ');
    expect(window.sendAction).not.toHaveBeenCalled();
  });
});

// ── ph-ship-picker (lobby) ───────────────────────────────────────────────────

describe('ph-ship-picker (lobby) keyboard contract', () => {
  const state = {
    ships: [
      { template_path: 'alpha.toml', label: 'Alpha Hull', class: 'destroyer', hull_id: '7', power_rating: 5, station_count: 4 },
      { template_path: 'bravo.toml', name: 'Bravo Hull', class: 'cruiser' },
    ],
    pendingTemplate: null,
  };

  it('is a named listbox host (AC: role + name)', () => {
    const el = mount('ph-ship-picker', state);
    expect(el.getAttribute('role')).toBe('listbox');
    expect(el.getAttribute('aria-label')).toBe(t('component.ship_picker.label'));
  });

  it('renders native <button role="option"> cards named by their hull name', () => {
    const el = mount('ph-ship-picker', state);
    const cards = options(el, '.ship-card');
    expect(cards).toHaveLength(2);
    for (const c of cards) {
      expect(c.tagName).toBe('BUTTON');
      expect(c.getAttribute('role')).toBe('option');
    }
    expect(cards[0].textContent).toContain('Alpha Hull');
    expect(cards[1].textContent).toContain('Bravo Hull');
    expect(singleTabStop(cards)).toBe(true);
  });

  it('roves the single Tab stop with the arrow keys', () => {
    const el = mount('ph-ship-picker', state);
    const cards = options(el, '.ship-card');
    cards[0].focus();
    keydown(cards[0], 'ArrowRight');
    expect(el.shadowRoot.activeElement).toBe(cards[1]);
    expect(cards[1].tabIndex).toBe(0);
  });

  it('a click dispatches the ship-selected event; the roving keydown does not fork it', () => {
    const el = mount('ph-ship-picker', state);
    const cards = options(el, '.ship-card');
    const seen = [];
    el.addEventListener('ship-selected', (e) => seen.push(e.detail.template_path));

    cards[0].click();
    expect(seen).toEqual(['alpha.toml']);

    cards[0].focus();
    keydown(cards[0], 'Enter');
    keydown(cards[0], ' ');
    expect(seen).toEqual(['alpha.toml']);   // no second, forked dispatch
  });

  it('marks the pending pick selected and disables the other cards for keyboard parity', () => {
    const el = mount('ph-ship-picker', { ...state, pendingTemplate: 'alpha.toml' });
    const cards = options(el, '.ship-card');
    expect(cards[0].getAttribute('aria-selected')).toBe('true');
    expect(cards[0].disabled).toBe(false);
    expect(cards[1].getAttribute('aria-selected')).toBe('false');
    expect(cards[1].disabled).toBe(true);
    // The single Tab stop settles on the still-enabled (pending) card.
    expect(cards[0].tabIndex).toBe(0);
    expect(cards[1].tabIndex).toBe(-1);
  });
});
