/**
 * gui/settings-tabs.js — the settings menu's tab list, shared by both pages.
 *
 * The host page's cog (issue #939, `gui/server-settings.js`) and the phone
 * client's (issue #940, `gui/settings-panel.js`) offer the same three tabs and
 * hide the same one in the public demo build. That agreement is the whole
 * point: a player looking at a phone and the host looking at the viewscreen
 * must not disagree about which controls exist.
 *
 * So the list lives here rather than twice, and `server-settings.js` re-exports
 * it under its original names so nothing that already imported it had to move.
 *
 * DOM-free and window-free at import time, so vitest can import it in Node.
 */

/**
 * The three tabs, in display order.
 *
 * `gated` tabs vanish in the demo build. Only Debug/Cheat is gated: Audio and
 * Gameplay must keep working in the demo, which is why nothing built for those
 * two tabs may reach for debug-only plumbing.
 *
 * A tab surviving the demo build does NOT mean every control on it does. The
 * phone's Gameplay tab keeps its station controls there and hides its pause,
 * which is a control-level gate `gui/settings-panel.js` owns — see
 * `ClientMessage::TogglePause` in `src/core/messages.rs` for why that one is
 * decided per control rather than per tab.
 */
export const TABS = [
  { id: 'debug', labelId: 'settings.tab.debug', gated: true },
  { id: 'audio', labelId: 'settings.tab.audio', gated: false },
  { id: 'gameplay', labelId: 'settings.tab.gameplay', gated: false },
];

/**
 * Which tabs this build actually shows.
 *
 * @param {boolean} demo — true in the public demo build.
 * @returns {Array<{id: string, labelId: string, gated: boolean}>}
 */
export function visibleTabs(demo) {
  return TABS.filter((tab) => !(tab.gated && demo));
}

/**
 * The tab to show, given a previously-selected one that may no longer exist.
 *
 * Extracted because both pages need the same answer and getting it wrong is
 * invisible: a panel whose active tab was gated away renders an empty body
 * rather than falling back, which reads as "the settings menu is broken".
 *
 * @param {string|null} wanted — the currently-selected tab id, if any.
 * @param {boolean} demo
 * @returns {string|null} null only when every tab is gated away.
 */
export function resolveActiveTab(wanted, demo) {
  const tabs = visibleTabs(demo);
  if (tabs.some((tab) => tab.id === wanted)) return wanted;
  return tabs.length > 0 ? tabs[0].id : null;
}
