import { describe, it, expect, vi } from 'vitest';
import {
  tabBarLayout,
  renderTabBar,
  useInitials,
  currentOrientation,
  CONSOLE_LABEL,
  CONSOLE_INITIAL,
  INITIALS_THRESHOLD,
} from '../../gui/tab-bar.js';

// Minimal stand-in for `document.createElement('button')` since Vitest runs
// in Node without a DOM. We construct a tiny fake document so renderTabBar's
// behaviour can be asserted without pulling in jsdom.
function fakeDoc() {
  function makeEl(tag) {
    const el = {
      tagName: tag.toUpperCase(),
      children: [],
      attrs: {},
      dataset: {},
      style: {},
      listeners: {},
      firstChild: null,
      ownerDocument: null,
      type: '',
      className: '',
      textContent: '',
      get childNodes() { return this.children; },
      setAttribute(k, v) { this.attrs[k] = v; },
      getAttribute(k) { return this.attrs[k]; },
      appendChild(child) {
        this.children.push(child);
        this.firstChild = this.children[0] || null;
      },
      removeChild(child) {
        this.children = this.children.filter((c) => c !== child);
        this.firstChild = this.children[0] || null;
      },
      addEventListener(ev, fn) {
        (this.listeners[ev] = this.listeners[ev] || []).push(fn);
      },
      click() {
        for (const fn of (this.listeners.click || [])) fn();
      },
    };
    return el;
  }
  const doc = { createElement: (tag) => { const e = makeEl(tag); e.ownerDocument = doc; return e; } };
  return doc;
}

function fakeRoot() {
  const doc = fakeDoc();
  const root = doc.createElement('div');
  return root;
}

describe('CONSOLE_LABEL / CONSOLE_INITIAL', () => {
  it('keys all nine consoles', () => {
    const expected = [
      'CaptainChair', 'Helm', 'Tactical', 'Repair', 'Sensors',
      'Shields', 'Navigation', 'Power', 'Comms',
    ];
    expect(Object.keys(CONSOLE_LABEL).sort()).toEqual(expected.slice().sort());
    expect(Object.keys(CONSOLE_INITIAL).sort()).toEqual(expected.slice().sort());
  });

  it('label for CaptainChair is "Captain\'s Chair"', () => {
    expect(CONSOLE_LABEL.CaptainChair).toBe("Captain's Chair");
  });

  it('initial for Shields is "SH"', () => {
    expect(CONSOLE_INITIAL.Shields).toBe('SH');
  });

  it('initial for Tactical is "T"', () => {
    expect(CONSOLE_INITIAL.Tactical).toBe('T');
  });

  it('label and initial maps are frozen', () => {
    expect(Object.isFrozen(CONSOLE_LABEL)).toBe(true);
    expect(Object.isFrozen(CONSOLE_INITIAL)).toBe(true);
  });

  it('exports INITIALS_THRESHOLD === 5', () => {
    expect(INITIALS_THRESHOLD).toBe(5);
  });
});

describe('currentOrientation', () => {
  it('returns landscape when width > height', () => {
    expect(currentOrientation({ innerWidth: 1024, innerHeight: 768 })).toBe('landscape');
  });

  it('returns portrait when height > width', () => {
    expect(currentOrientation({ innerWidth: 414, innerHeight: 896 })).toBe('portrait');
  });

  it('returns portrait when width === height (square ties go to portrait)', () => {
    expect(currentOrientation({ innerWidth: 800, innerHeight: 800 })).toBe('portrait');
  });

  it('defaults to portrait when window is missing', () => {
    expect(currentOrientation(null)).toBe('portrait');
    expect(currentOrientation({})).toBe('portrait');
  });
});

describe('useInitials', () => {
  it('returns false for landscape regardless of count', () => {
    expect(useInitials(['A','B','C','D','E','F','G'], 'landscape')).toBe(false);
  });

  it('returns false for portrait with < 5 consoles', () => {
    expect(useInitials([], 'portrait')).toBe(false);
    expect(useInitials(['A'], 'portrait')).toBe(false);
    expect(useInitials(['A','B','C','D'], 'portrait')).toBe(false);
  });

  it('returns true for portrait with exactly 5 consoles (threshold inclusive)', () => {
    expect(useInitials(['A','B','C','D','E'], 'portrait')).toBe(true);
  });

  it('returns true for portrait with > 5 consoles', () => {
    expect(useInitials(['A','B','C','D','E','F','G','H','I'], 'portrait')).toBe(true);
  });

  it('returns false when consoles is not an array', () => {
    expect(useInitials(null, 'portrait')).toBe(false);
    expect(useInitials(undefined, 'portrait')).toBe(false);
    expect(useInitials('CaptainChair', 'portrait')).toBe(false);
  });
});

describe('tabBarLayout — hidden conditions', () => {
  it('hides when not in-game (lobby)', () => {
    const out = tabBarLayout(['CaptainChair', 'Tactical'], 'CaptainChair', 'portrait', false);
    expect(out.hidden).toBe(true);
  });

  it('hides when in-game with no consoles', () => {
    const out = tabBarLayout([], null, 'portrait', true);
    expect(out.hidden).toBe(true);
  });

  it('hides when in-game with exactly 1 console (single-console players see no tabs)', () => {
    const out = tabBarLayout(['CaptainChair'], 'CaptainChair', 'portrait', true);
    expect(out.hidden).toBe(true);
  });

  it('shows when in-game with 2 consoles', () => {
    const out = tabBarLayout(['CaptainChair', 'Tactical'], 'CaptainChair', 'portrait', true);
    expect(out.hidden).toBe(false);
  });

  it('defaults to portrait when orientation is unknown', () => {
    const out = tabBarLayout(['CaptainChair', 'Tactical'], 'CaptainChair', 'oblique', true);
    expect(out.orientation).toBe('portrait');
  });
});

describe('tabBarLayout — labels', () => {
  it('uses full names for 2 consoles in portrait', () => {
    const out = tabBarLayout(['CaptainChair', 'Tactical'], 'CaptainChair', 'portrait', true);
    expect(out.useInitials).toBe(false);
    expect(out.buttons.map((b) => b.label)).toEqual(["Captain's Chair", 'Tactical']);
  });

  it('uses full names for 4 consoles in portrait (boundary just below threshold)', () => {
    const out = tabBarLayout(
      ['CaptainChair', 'Tactical', 'Repair', 'Helm'],
      'Helm', 'portrait', true,
    );
    expect(out.useInitials).toBe(false);
    expect(out.buttons.map((b) => b.label)).toEqual([
      "Captain's Chair", 'Tactical', 'Repair', 'Helm',
    ]);
  });

  it('uses initials for 5 consoles in portrait', () => {
    const out = tabBarLayout(
      ['CaptainChair', 'Helm', 'Tactical', 'Repair', 'Sensors'],
      'Tactical', 'portrait', true,
    );
    expect(out.useInitials).toBe(true);
    expect(out.buttons.map((b) => b.label)).toEqual(['CC', 'H', 'T', 'R', 'S']);
  });

  it('uses full names for 9 consoles in landscape (vertical bar has room)', () => {
    const all = ['CaptainChair','Helm','Tactical','Repair','Sensors','Shields','Navigation','Power','Comms'];
    const out = tabBarLayout(all, 'Comms', 'landscape', true);
    expect(out.useInitials).toBe(false);
    expect(out.buttons.map((b) => b.label)).toEqual([
      "Captain's Chair", 'Helm', 'Tactical', 'Repair', 'Sensors',
      'Shields', 'Navigation', 'Power', 'Comms',
    ]);
  });
});

describe('tabBarLayout — active highlight', () => {
  it('marks the active button as active=true and others as false', () => {
    const out = tabBarLayout(['CaptainChair', 'Tactical', 'Repair'], 'Tactical', 'portrait', true);
    expect(out.buttons.map((b) => b.active)).toEqual([false, true, false]);
  });

  it('leaves no button active when active is null', () => {
    const out = tabBarLayout(['CaptainChair', 'Tactical'], null, 'portrait', true);
    expect(out.buttons.every((b) => b.active === false)).toBe(true);
  });

  it('leaves no button active when active is not in the list', () => {
    const out = tabBarLayout(['CaptainChair', 'Tactical'], 'Repair', 'portrait', true);
    expect(out.buttons.every((b) => b.active === false)).toBe(true);
  });
});

describe('renderTabBar — DOM mutations', () => {
  it('returns the layout untouched when root is null', () => {
    const layout = tabBarLayout(['CaptainChair', 'Tactical'], 'CaptainChair', 'portrait', true);
    expect(renderTabBar(null, layout)).toBe(layout);
  });

  it('sets aria-hidden=true and clears children when layout is hidden (does not touch inline style — CSS drives display via [aria-hidden])', () => {
    const root = fakeRoot();
    const layout = tabBarLayout(['CaptainChair'], 'CaptainChair', 'portrait', true);
    renderTabBar(root, layout);
    expect(root.getAttribute('aria-hidden')).toBe('true');
    // No inline style.display is set — CSS rule [aria-hidden="true"] handles it.
    expect(root.style.display).toBeUndefined();
    expect(root.children.length).toBe(0);
  });

  it('sets aria-hidden=false and sets orientation dataset and renders one button per console', () => {
    const root = fakeRoot();
    const layout = tabBarLayout(['CaptainChair', 'Tactical', 'Repair'], 'Tactical', 'portrait', true);
    renderTabBar(root, layout);
    expect(root.getAttribute('aria-hidden')).toBe('false');
    expect(root.dataset.orientation).toBe('portrait');
    expect(root.children.length).toBe(3);
    const btns = root.children;
    expect(btns[0].textContent).toBe("Captain's Chair");
    expect(btns[1].textContent).toBe('Tactical');
    expect(btns[2].textContent).toBe('Repair');
  });

  it('marks the active button with the .active class, role=tab, and aria-selected=true', () => {
    const root = fakeRoot();
    const layout = tabBarLayout(['CaptainChair', 'Tactical', 'Repair'], 'Tactical', 'portrait', true);
    renderTabBar(root, layout);
    const active = root.children.filter((b) => b.className.includes('active'));
    expect(active.length).toBe(1);
    expect(active[0].dataset.console).toBe('Tactical');
    expect(active[0].getAttribute('role')).toBe('tab');
    expect(active[0].getAttribute('aria-selected')).toBe('true');
    // Non-active buttons get role=tab + aria-selected=false (not aria-pressed).
    const inactive = root.children.filter((b) => !b.className.includes('active'));
    for (const b of inactive) {
      expect(b.getAttribute('role')).toBe('tab');
      expect(b.getAttribute('aria-selected')).toBe('false');
      expect(b.getAttribute('aria-pressed')).toBeUndefined();
    }
  });

  it('sets dataset.console on every button to the console name', () => {
    const root = fakeRoot();
    const layout = tabBarLayout(['CaptainChair', 'Tactical', 'Repair'], 'Repair', 'landscape', true);
    renderTabBar(root, layout);
    expect(root.children.map((c) => c.dataset.console))
      .toEqual(['CaptainChair', 'Tactical', 'Repair']);
  });

  it('wires onPress click handlers that fire with the console name', () => {
    const root = fakeRoot();
    const onPress = vi.fn();
    const layout = tabBarLayout(['CaptainChair', 'Tactical'], 'CaptainChair', 'portrait', true);
    renderTabBar(root, layout, { onPress });
    root.children[1].click();
    expect(onPress).toHaveBeenCalledExactlyOnceWith('Tactical');
  });

  it('does not throw when onPress is omitted', () => {
    const root = fakeRoot();
    const layout = tabBarLayout(['CaptainChair', 'Tactical'], 'CaptainChair', 'portrait', true);
    renderTabBar(root, layout);
    expect(() => root.children[0].click()).not.toThrow();
  });

  it('rebuilds from scratch on each call (no stale buttons)', () => {
    const root = fakeRoot();
    renderTabBar(root, tabBarLayout(['CaptainChair', 'Tactical', 'Repair'], 'Tactical', 'portrait', true));
    expect(root.children.length).toBe(3);
    renderTabBar(root, tabBarLayout(['CaptainChair', 'Helm'], 'Helm', 'portrait', true));
    expect(root.children.length).toBe(2);
    expect(root.children.map((c) => c.dataset.console)).toEqual(['CaptainChair', 'Helm']);
  });

  it('flipping from shown to hidden clears the buttons and flips aria-hidden', () => {
    const root = fakeRoot();
    renderTabBar(root, tabBarLayout(['CaptainChair', 'Tactical'], 'CaptainChair', 'portrait', true));
    expect(root.children.length).toBe(2);
    expect(root.getAttribute('aria-hidden')).toBe('false');
    renderTabBar(root, tabBarLayout(['CaptainChair'], 'CaptainChair', 'portrait', true));
    expect(root.children.length).toBe(0);
    expect(root.getAttribute('aria-hidden')).toBe('true');
  });
});
