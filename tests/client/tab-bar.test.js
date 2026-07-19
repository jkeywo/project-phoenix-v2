import { t } from '../../gui/strings.js';
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
    let _textContent = '';
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
      get textContent() {
        if (this.children.length === 0) return _textContent;
        return this.children.map((c) => c.textContent).join('');
      },
      set textContent(v) { _textContent = v; },
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
        for (const fn of (this.listeners.click || [])) fn({ preventDefault: () => {} });
      },
      pointerdown() {
        for (const fn of (this.listeners.pointerdown || [])) fn({ preventDefault: () => {} });
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
  it('keys all consoles by lowercase station id', () => {
    const expected = [
      'captain', 'helm', 'tactical', 'repair', 'sensors',
      'shields', 'navigation', 'power', 'comms', 'science', 'engineering',
    ];
    expect(Object.keys(CONSOLE_LABEL).sort()).toEqual(expected.slice().sort());
    expect(Object.keys(CONSOLE_INITIAL).sort()).toEqual(expected.slice().sort());
  });

  it('label for captain is "Captain\'s Chair"', () => {
    expect(CONSOLE_LABEL.captain).toBe(t('console_label.captain'));
  });

  it('initial for shields is "SH"', () => {
    expect(CONSOLE_INITIAL.shields).toBe(t('console_initial.shields'));
  });

  it('initial for tactical is "T"', () => {
    expect(CONSOLE_INITIAL.tactical).toBe(t('console_initial.tactical'));
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
    expect(useInitials('captain', 'portrait')).toBe(false);
  });
});

describe('tabBarLayout — hidden conditions', () => {
  it('hides when not in-game (lobby)', () => {
    const out = tabBarLayout(['captain', 'tactical'], 'captain', 'portrait', false);
    expect(out.hidden).toBe(true);
  });

  it('hides when in-game with no consoles', () => {
    const out = tabBarLayout([], null, 'portrait', true);
    expect(out.hidden).toBe(true);
  });

  it('hides when in-game with exactly 1 console (single-station mode — full screen)', () => {
    const out = tabBarLayout(['captain'], 'captain', 'portrait', true);
    expect(out.hidden).toBe(true);
    expect(out.buttons).toHaveLength(0);
  });

  it('shows when in-game with 2 consoles', () => {
    const out = tabBarLayout(['captain', 'tactical'], 'captain', 'portrait', true);
    expect(out.hidden).toBe(false);
  });

  it('defaults to portrait when orientation is unknown', () => {
    const out = tabBarLayout(['captain', 'tactical'], 'captain', 'oblique', true);
    expect(out.orientation).toBe('portrait');
  });
});

describe('tabBarLayout — labels', () => {
  it('uses full names for 2 consoles in portrait', () => {
    const out = tabBarLayout(['captain', 'tactical'], 'captain', 'portrait', true);
    expect(out.useInitials).toBe(false);
    expect(out.buttons.map((b) => b.label)).toEqual([t('console_label.captain'), t('console_label.tactical')]);
  });

  it('uses full names for 4 consoles in portrait (boundary just below threshold)', () => {
    const out = tabBarLayout(
      ['captain', 'tactical', 'repair', 'helm'],
      'helm', 'portrait', true,
    );
    expect(out.useInitials).toBe(false);
    expect(out.buttons.map((b) => b.label)).toEqual([
      t('console_label.captain'), t('console_label.tactical'), t('console_label.repair'), t('console_label.helm'),
    ]);
  });

  it('uses initials for 5 consoles in portrait', () => {
    const out = tabBarLayout(
      ['captain', 'helm', 'tactical', 'repair', 'sensors'],
      'tactical', 'portrait', true,
    );
    expect(out.useInitials).toBe(true);
    expect(out.buttons.map((b) => b.label)).toEqual(['captain', 'helm', 'tactical', 'repair', 'sensors'].map((c) => t('console_initial.' + c)));
  });

  it('uses full names for 9 consoles in landscape (horizontal bar has room)', () => {
    const all = ['captain','helm','tactical','repair','sensors','shields','navigation','power','comms'];
    const out = tabBarLayout(all, 'comms', 'landscape', true);
    expect(out.useInitials).toBe(false);
    expect(out.buttons.map((b) => b.label)).toEqual([
      ...['captain', 'helm', 'tactical', 'repair', 'sensors', 'shields', 'navigation', 'power', 'comms'].map((c) => t('console_label.' + c)),
    ]);
  });
});

describe('tabBarLayout — active highlight', () => {
  it('marks the active button as active=true and others as false', () => {
    const out = tabBarLayout(['captain', 'tactical', 'repair'], 'tactical', 'portrait', true);
    expect(out.buttons.map((b) => b.active)).toEqual([false, true, false]);
  });

  it('leaves no button active when active is null', () => {
    const out = tabBarLayout(['captain', 'tactical'], null, 'portrait', true);
    expect(out.buttons.every((b) => b.active === false)).toBe(true);
  });

  it('leaves no button active when active is not in the list', () => {
    const out = tabBarLayout(['captain', 'tactical'], 'repair', 'portrait', true);
    expect(out.buttons.every((b) => b.active === false)).toBe(true);
  });
});

describe('tabBarLayout — compactActive mode', () => {
  it('shows all buttons when compactActive is false (default)', () => {
    const out = tabBarLayout(['captain', 'helm', 'tactical'], 'helm', 'portrait', true, null, false);
    expect(out.buttons).toHaveLength(3);
    expect(out.buttons.map((b) => b.console)).toEqual(['captain', 'helm', 'tactical']);
  });

  it('shows all buttons when compactActive is omitted (backward compat)', () => {
    const out = tabBarLayout(['captain', 'helm', 'tactical'], 'helm', 'portrait', true);
    expect(out.buttons).toHaveLength(3);
  });

  it('shows only the active button when compactActive is true and active is set', () => {
    const out = tabBarLayout(['captain', 'helm', 'tactical'], 'helm', 'portrait', true, null, true);
    expect(out.buttons).toHaveLength(1);
    expect(out.buttons[0].console).toBe('helm');
    expect(out.buttons[0].active).toBe(true);
  });

  it('shows only the active button with correct label in compact mode', () => {
    const out = tabBarLayout(['captain', 'helm', 'tactical'], 'captain', 'portrait', true, null, true);
    expect(out.buttons).toHaveLength(1);
    expect(out.buttons[0].label).toBe(t('console_label.captain'));
    expect(out.buttons[0].active).toBe(true);
  });

  it('shows all buttons when compactActive is true but active is null', () => {
    const out = tabBarLayout(['captain', 'helm', 'tactical'], null, 'portrait', true, null, true);
    expect(out.buttons).toHaveLength(3);
  });

  it('shows all buttons when compactActive is true but active is not in the list', () => {
    const out = tabBarLayout(['captain', 'helm'], 'repair', 'portrait', true, null, true);
    expect(out.buttons).toHaveLength(2);
  });

  it('preserves hullPct on the single compact button', () => {
    // Post issue #618/#619 the `consoles` list and each hull entry's
    // `system_id` are both lowercase station ids; matching is direct.
    const hull = [{ system_id: 'helm', current: 40, max_hp: 100 }];
    const out = tabBarLayout(['captain', 'helm', 'tactical'], 'helm', 'portrait', true, hull, true);
    expect(out.buttons).toHaveLength(1);
    expect(out.buttons[0].hullPct).toBe(40);
  });

  it('keeps bar visible (not hidden) in compact mode with 2+ consoles', () => {
    const out = tabBarLayout(['captain', 'helm', 'tactical'], 'helm', 'portrait', true, null, true);
    expect(out.hidden).toBe(false);
  });

  it('shows initials in portrait at >= 5 consoles even in compact mode', () => {
    const all = ['captain', 'helm', 'tactical', 'repair', 'sensors', 'shields'];
    const out = tabBarLayout(all, 'tactical', 'portrait', true, null, true);
    expect(out.useInitials).toBe(true);
    expect(out.buttons).toHaveLength(1);
    expect(out.buttons[0].label).toBe(t('console_initial.tactical'));
  });
});

describe('renderTabBar — DOM mutations', () => {
  it('returns the layout untouched when root is null', () => {
    const layout = tabBarLayout(['captain', 'tactical'], 'captain', 'portrait', true);
    expect(renderTabBar(null, layout)).toBe(layout);
  });

  it('sets aria-hidden=true and clears children when layout is hidden (not in-game)', () => {
    const root = fakeRoot();
    const layout = tabBarLayout(['captain'], 'captain', 'portrait', false);
    renderTabBar(root, layout);
    expect(root.getAttribute('aria-hidden')).toBe('true');
    // No inline style.display is set — CSS rule [aria-hidden="true"] handles it.
    expect(root.style.display).toBeUndefined();
    expect(root.children.length).toBe(0);
  });

  it('sets aria-hidden=true for single-console player in-game (single-station mode)', () => {
    const root = fakeRoot();
    const layout = tabBarLayout(['captain'], 'captain', 'portrait', true);
    renderTabBar(root, layout);
    expect(root.getAttribute('aria-hidden')).toBe('true');
    expect(root.children.length).toBe(0);
  });

  it('sets aria-hidden=false and sets orientation dataset and renders one button per console', () => {
    const root = fakeRoot();
    const layout = tabBarLayout(['captain', 'tactical', 'repair'], 'tactical', 'portrait', true);
    renderTabBar(root, layout);
    expect(root.getAttribute('aria-hidden')).toBe('false');
    expect(root.dataset.orientation).toBe('portrait');
    expect(root.children.length).toBe(3);
    const btns = root.children;
    expect(btns[0].textContent).toBe(t('console_label.captain'));
    expect(btns[1].textContent).toBe(t('console_label.tactical'));
    expect(btns[2].textContent).toBe(t('console_label.repair'));
  });

  it('marks the active button with the .active class, role=tab, and aria-selected=true', () => {
    const root = fakeRoot();
    const layout = tabBarLayout(['captain', 'tactical', 'repair'], 'tactical', 'portrait', true);
    renderTabBar(root, layout);
    const active = root.children.filter((b) => b.className.includes('active'));
    expect(active.length).toBe(1);
    expect(active[0].dataset.console).toBe('tactical');
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
    const layout = tabBarLayout(['captain', 'tactical', 'repair'], 'repair', 'landscape', true);
    renderTabBar(root, layout);
    expect(root.children.map((c) => c.dataset.console))
      .toEqual(['captain', 'tactical', 'repair']);
  });

  it('wires onPress pointerdown handlers that fire with the console name', () => {
    const root = fakeRoot();
    const onPress = vi.fn();
    const layout = tabBarLayout(['captain', 'tactical'], 'captain', 'portrait', true);
    renderTabBar(root, layout, { onPress });
    root.children[1].pointerdown();
    expect(onPress).toHaveBeenCalledExactlyOnceWith('tactical');
  });

  it('does not throw when onPress is omitted', () => {
    const root = fakeRoot();
    const layout = tabBarLayout(['captain', 'tactical'], 'captain', 'portrait', true);
    renderTabBar(root, layout);
    expect(() => root.children[0].pointerdown()).not.toThrow();
  });

  it('rebuilds from scratch on each call (no stale buttons)', () => {
    const root = fakeRoot();
    renderTabBar(root, tabBarLayout(['captain', 'tactical', 'repair'], 'tactical', 'portrait', true));
    expect(root.children.length).toBe(3);
    renderTabBar(root, tabBarLayout(['captain', 'helm'], 'helm', 'portrait', true));
    expect(root.children.length).toBe(2);
    expect(root.children.map((c) => c.dataset.console)).toEqual(['captain', 'helm']);
  });

  it('reducing to 1 console hides the bar (single-station mode)', () => {
    const root = fakeRoot();
    renderTabBar(root, tabBarLayout(['captain', 'tactical'], 'captain', 'portrait', true));
    expect(root.children.length).toBe(2);
    expect(root.getAttribute('aria-hidden')).toBe('false');
    renderTabBar(root, tabBarLayout(['captain'], 'captain', 'portrait', true));
    expect(root.children.length).toBe(0);
    expect(root.getAttribute('aria-hidden')).toBe('true');
  });

  it('going out of game hides the bar', () => {
    const root = fakeRoot();
    renderTabBar(root, tabBarLayout(['captain', 'tactical'], 'captain', 'portrait', true));
    expect(root.getAttribute('aria-hidden')).toBe('false');
    renderTabBar(root, tabBarLayout(['captain', 'tactical'], 'captain', 'portrait', false));
    expect(root.getAttribute('aria-hidden')).toBe('true');
  });
});
