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

export const CONSOLE_LABEL = Object.freeze({
  CaptainChair: "Captain's Chair",
  Helm: 'Helm',
  Tactical: 'Tactical',
  Repair: 'Repair',
  Sensors: 'Sensors',
  Shields: 'Shields',
  Navigation: 'Navigation',
  Power: 'Power',
  Comms: 'Comms',
});

export const CONSOLE_INITIAL = Object.freeze({
  CaptainChair: 'CC',
  Helm: 'H',
  Tactical: 'T',
  Repair: 'R',
  Sensors: 'S',
  Shields: 'SH',
  Navigation: 'N',
  Power: 'P',
  Comms: 'C',
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
//   consoles    — string[] of Console enum names owned by the local player
//   active      — currently-selected console name (or null)
//   orientation — 'portrait' | 'landscape'
//   inGame      — boolean; tab bar is hidden in the lobby
//   consoleHull — optional [{ console, current, max_hp }] from server state
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
export function tabBarLayout(consoles, active, orientation, inGame, consoleHull) {
  const list = Array.isArray(consoles) ? consoles : [];
  const orient = orientation === 'landscape' ? 'landscape' : 'portrait';
  const initials = useInitials(list, orient);
  // Hide when not in-game or when there are no consoles.
  // Single-console players still see the bar (for the title label).
  const hidden = !inGame || list.length === 0;
  // Build a lookup from console name → hull pct for damageable consoles.
  const hullMap = {};
  if (Array.isArray(consoleHull)) {
    for (const h of consoleHull) {
      if (h && h.console && h.max_hp > 0) {
        hullMap[h.console] = Math.max(0, Math.min(100, (h.current / h.max_hp) * 100));
      }
    }
  }
  // Only render tab buttons when there are 2+ consoles to switch between.
  const buttons = list.length >= 2 ? list.map((c) => ({
    console: c,
    label: initials ? (CONSOLE_INITIAL[c] || c) : (CONSOLE_LABEL[c] || c),
    active: c === active,
    hullPct: hullMap[c] !== undefined ? hullMap[c] : null,
  })) : [];
  return { hidden, orientation: orient, useInitials: initials, buttons };
}

// DOM renderer. Rebuilds the tab-bar root from scratch on each call. Cheap
// (at most 9 buttons) and avoids the bug of stale state from partial diffs.
//
//   root      — HTMLElement (the #console-tab-bar container)
//   layout    — output of tabBarLayout()
//   options   — { onPress: (consoleName) => void }
//
// Side effects (no inline `style.display` — visibility is cascade-driven
// by the `[aria-hidden="true"] { display: none }` rule in client.html):
//   - root.setAttribute('aria-hidden', ...) per layout.hidden
//   - root.dataset.orientation set to the layout orientation (drives CSS)
//   - root.innerHTML rebuilt with <button role="tab" class="tab-button [active]">
//     for each entry. Each button's click handler calls options.onPress.
//
// Returns the layout (handy for tests that want to assert on the structure).
export function renderTabBar(root, layout, options) {
  if (!root) return layout;
  const opts = options || {};
  root.setAttribute('aria-hidden', layout.hidden ? 'true' : 'false');
  root.dataset.orientation = layout.orientation;
  // Rebuild children.
  while (root.firstChild) root.removeChild(root.firstChild);
  if (layout.hidden) return layout;
  for (const btn of layout.buttons) {
    const el = (root.ownerDocument || document).createElement('button');
    el.type = 'button';
    el.className = 'tab-button' + (btn.active ? ' active' : '');
    el.dataset.console = btn.console;
    // role="tab" + aria-selected matches the role="tablist" container on
    // the parent (ARIA contract — toggle buttons would use aria-pressed,
    // but inside a tablist screen readers expect tabs with aria-selected).
    el.setAttribute('role', 'tab');
    el.setAttribute('aria-selected', btn.active ? 'true' : 'false');
    const labelEl = (root.ownerDocument || document).createElement('span');
    labelEl.className = 'tab-label';
    labelEl.textContent = btn.label;
    el.appendChild(labelEl);
    if (btn.hullPct !== null) {
      const bar = (root.ownerDocument || document).createElement('span');
      bar.className = 'tab-hull-bar';
      bar.style.setProperty('--hull-pct', btn.hullPct + '%');
      const color = btn.hullPct > 60 ? '#4caf50' : btn.hullPct > 25 ? '#f59e0b' : '#ef4444';
      bar.style.setProperty('--hull-color', color);
      el.appendChild(bar);
    }
    if (typeof opts.onPress === 'function') {
      // pointerdown fires immediately on touch (no 300 ms click-delay).
      el.addEventListener('pointerdown', (e) => { e.preventDefault(); opts.onPress(btn.console); });
    }
    root.appendChild(el);
  }
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
