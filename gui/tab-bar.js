// Tab bar for the client GUI shell. Pure layout + DOM renderer (issue #441).
//
// The tab bar lets a player switch between consoles assigned to them. Layout
// is always a horizontal strip across the top, left-aligned:
//
//   - Hidden when `inGame === false` OR when the player owns <= 1 console.
//   - Portrait: smaller buttons, uses initials when count >= 5.
//   - Landscape: full-size buttons with full labels.
//
// This module exports both pure functions (testable with no DOM) and a
// DOM-mutating renderer. The pure functions feed Vitest tests; the renderer
// is consumed by the inline `<script>` in `client.html`.
//
// Post issue #619 the `consoles` input list carries lowercase station ids
// (matching `StationId`). Pre-#619 it carried PascalCase Console enum names.

export const CONSOLE_LABEL = Object.freeze({
  captain: "Captain's Chair",
  helm: 'Helm',
  tactical: 'Tactical',
  repair: 'Repair',
  sensors: 'Sensors',
  science: 'Science',
  shields: 'Shields',
  navigation: 'Navigation',
  power: 'Power',
  comms: 'Comms',
});

export const CONSOLE_INITIAL = Object.freeze({
  captain: 'CC',
  helm: 'H',
  tactical: 'T',
  repair: 'R',
  sensors: 'S',
  science: 'SC',
  shields: 'SH',
  navigation: 'N',
  power: 'P',
  comms: 'C',
});

// Threshold at which the portrait bar collapses to initials. Matches the
// Rust `console_shell::rebuild_embedded_tab_bars` rule (>= 5 consoles).
export const INITIALS_THRESHOLD = 5;

// Returns 'portrait' or 'landscape' from `window.innerWidth / innerHeight`.
// Pure function that takes an explicit window-like object so Vitest can
// substitute a fake. Defaults to the real window when present.
export function currentOrientation(win) {
  const w = win || (typeof window !== 'undefined' ? window : null);
  if (!w || typeof w.innerWidth !== 'number' || typeof w.innerHeight !== 'number') {
    return 'portrait';
  }
  return w.innerWidth > w.innerHeight ? 'landscape' : 'portrait';
}

// True if the portrait bar should show initials instead of full names.
// Landscape has more horizontal room so uses full labels.
export function useInitials(consoles, orientation) {
  if (!Array.isArray(consoles)) return false;
  if (orientation !== 'portrait') return false;
  return consoles.length >= INITIALS_THRESHOLD;
}

// Pure: produce a description of the tab bar to render.
//   consoles    — string[] of lowercase station ids owned by the local player
//   active      — currently-selected station id (or null)
//   orientation — 'portrait' | 'landscape'
//   inGame      — boolean; tab bar is hidden in the lobby
//   consoleHull — optional [{ system_id, current, max_hp, ... }] from server
//                 state (post issue #618).
//   compactActive — boolean; when true and a console is active, only show
//                   the active console's button (others are hidden)
// Returns:
//   {
//     hidden: bool,
//     orientation: 'portrait'|'landscape',
//     useInitials: bool,
//     buttons: [{ console, label, active, hullPct }],
//   }
// hullPct is 0-100 for damageable consoles, null for non-damageable.
// Tab is always a horizontal strip across the top; orientation only affects
// button size and whether initials or full labels are used.
export function tabBarLayout(consoles, active, orientation, inGame, consoleHull, compactActive) {
  const list = Array.isArray(consoles) ? consoles : [];
  const orient = orientation === 'landscape' ? 'landscape' : 'portrait';
  const initials = useInitials(list, orient);
  // Hide when not in-game or when the player owns <= 1 console (single-station
  // mode — full screen for the console, no tab bar needed).
  const hidden = !inGame || list.length <= 1;
  // Build a lookup from station id (lowercase) → hull pct for damageable
  // systems. Both the `consoles` list and the `system_id` on each hull
  // entry are lowercase station ids.
  const hullMap = {};
  if (Array.isArray(consoleHull)) {
    for (const h of consoleHull) {
      if (h && h.system_id && h.max_hp > 0) {
        hullMap[h.system_id] = Math.max(0, Math.min(100, (h.current / h.max_hp) * 100));
      }
    }
  }
  // Determine which consoles to render as tab buttons.
  // In compact mode with an active console, show only the active button.
  // Otherwise, only render buttons when there are 2+ consoles.
  let namesForButtons = [];
  if (compactActive && active && list.includes(active)) {
    namesForButtons = [active];
  } else if (list.length >= 2) {
    namesForButtons = list;
  }
  const buttons = namesForButtons.map((c) => {
    const hullPct = hullMap[c] !== undefined ? hullMap[c] : null;
    return {
      console: c,
      label: initials ? (CONSOLE_INITIAL[c] || c) : (CONSOLE_LABEL[c] || c),
      active: c === active,
      hullPct,
    };
  });
  return { hidden, orientation: orient, useInitials: initials, buttons };
}

// DOM renderer. Reconciled in-place by index (at most 9 buttons).
//
//   root      — HTMLElement (the #console-tab-bar container)
//   layout    — output of tabBarLayout()
//   options   — { onPress: (consoleName) => void }
//
// Side effects (no inline `style.display` — visibility is cascade-driven
// by the `[aria-hidden="true"] { display: none }` rule in client.html):
//   - root.setAttribute('aria-hidden', ...) per layout.hidden
//   - root.dataset.orientation set to the layout orientation (drives CSS)
//   - root children reconciled by index
//
// Returns the layout (handy for tests that want to assert on the structure).
export function renderTabBar(root, layout, options) {
  if (!root) return layout;
  const opts = options || {};
  root.setAttribute('aria-hidden', layout.hidden ? 'true' : 'false');
  root.dataset.orientation = layout.orientation;
  if (layout.hidden) {
    while (root.firstChild) root.removeChild(root.firstChild);
    return layout;
  }
  const btns = layout.buttons;
  while (root.children.length > btns.length) root.removeChild(root.children[root.children.length - 1]);
  while (root.children.length < btns.length) {
    const el = (root.ownerDocument || document).createElement('button');
    el.type = 'button'; el.setAttribute('role', 'tab');
    const labelEl = (root.ownerDocument || document).createElement('span');
    labelEl.className = 'tab-label'; el.appendChild(labelEl);
    const hullBar = (root.ownerDocument || document).createElement('span');
    hullBar.className = 'tab-hull-bar'; el.appendChild(hullBar);
    (function (btn) { btn.addEventListener('pointerdown', function (e) { e.preventDefault(); if (typeof btn._onPress === 'function') btn._onPress(btn.dataset.console); }); })(el);
    root.appendChild(el);
  }
  btns.forEach(function (btn, i) {
    var el = root.children[i];
    el.className = 'tab-button' + (btn.active ? ' active' : '');
    el.dataset.console = btn.console;
    el.setAttribute('aria-selected', btn.active ? 'true' : 'false');
    el._onPress = typeof opts.onPress === 'function' ? opts.onPress : null;
    el.firstChild.textContent = btn.label;
    var hullBar = el.children[1];
    if (btn.hullPct !== null) {
      hullBar.style.display = '';
      hullBar.style.setProperty('--hull-pct', btn.hullPct + '%');
      hullBar.style.setProperty('--hull-color', btn.hullPct > 60 ? '#4caf50' : btn.hullPct > 25 ? '#f59e0b' : '#ef4444');
    } else {
      hullBar.style.display = 'none';
    }
  });
  return layout;
}

// Expose for non-module scripts in `client.html`.
if (typeof window !== 'undefined') {
  window.tabBarLayout = tabBarLayout;
  window.renderTabBar = renderTabBar;
  window.useInitials = useInitials;
  window.currentOrientation = currentOrientation;
  window.CONSOLE_LABEL = CONSOLE_LABEL;
  window.CONSOLE_INITIAL = CONSOLE_INITIAL;
}
