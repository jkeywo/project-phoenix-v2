/**
 * gui/settings-tabs.js — the settings menu's tab list, shared by both pages.
 *
 * The host page's cog (issue #939, `gui/server-settings.js`) and the phone
 * client's (issue #940, `gui/settings-panel.js`) share the operational tabs
 * and hide the same debug tab in the public demo build. The phone also owns
 * two documentation tabs; they deliberately do not appear on the host.
 *
 * So the list lives here rather than twice, and `server-settings.js` re-exports
 * it under its original names so nothing that already imported it had to move.
 *
 * DOM-free and window-free at import time, so vitest can import it in Node.
 */

/**
 * The four shared tabs, in display order.
 *
 * Debug is LAST, deliberately: it is the developer/cheat tab, so the ordinary
 * Audio, Gameplay and Controls tabs come first, and the demo
 * build (where Debug is gated away entirely) opens on Audio rather than on a
 * blank where Debug used to be. `resolveActiveTab` falls back to `tabs[0]`, so
 * ordering Debug first would also have made it the default landing tab in dev.
 *
 * `gated` tabs vanish in the demo build. Only Debug/Cheat is gated: Audio and
 * Gameplay and Controls must keep working in the demo, which is why nothing
 * built for those tabs may reach for debug-only plumbing.
 *
 * A tab surviving the demo build does NOT mean every control on it does. The
 * phone's Gameplay tab keeps its station controls there and hides its pause,
 * which is a control-level gate `gui/settings-panel.js` owns — see
 * `ClientMessage::TogglePause` in `src/core/messages.rs` for why that one is
 * decided per control rather than per tab.
 */
export const TABS = [
  { id: 'audio', labelId: 'settings.tab.audio', gated: false },
  { id: 'gameplay', labelId: 'settings.tab.gameplay', gated: false },
  { id: 'controls', labelId: 'settings.tab.controls', gated: false },
  { id: 'debug', labelId: 'settings.tab.debug', gated: true },
];

/**
 * The phone client's Accessibility tab (issue #1102). PHONE-SCOPED on purpose:
 * the Accessibility profile belongs to the player / this pane, so it appears on
 * the phone client's cog and NOT on the shared host TABS above. Never gated —
 * it must survive the public demo build like the documentation tabs do.
 */
export const CLIENT_ACCESSIBILITY_TABS = [
  { id: 'accessibility', labelId: 'settings.tab.accessibility', gated: false },
];

/**
 * The Viewscreen's Display tab (issue #1427). VIEWSCREEN-SCOPED on purpose, and
 * for the mirror image of the reason the Accessibility tab above is phone-only:
 * what it edits is the ENDPOINT's record — this display's text size and
 * contrast, saved on this machine for whoever walks up to it next — rather than
 * one player's private profile. Two different records (see
 * `gui/viewscreen-presentation.js`), so two different tab ids, and a surface
 * therefore cannot show one tab and edit the other's store.
 *
 * Both viewscreen runtimes offer it — `server.html`'s cog and the native host
 * lobby's — and neither gates it: a shared screen in a demo build is still a
 * shared screen someone has to read from the back of the room.
 */
export const VIEWSCREEN_PRESENTATION_TABS = [
  { id: 'presentation', labelId: 'settings.tab.presentation', gated: false },
];

/** Client-only documentation tabs, always available including in demo builds. */
export const CLIENT_DOCUMENTATION_TABS = [
  { id: 'station-help', labelId: 'settings.tab.station_help', gated: false },
  { id: 'ship-manual', labelId: 'settings.tab.ship_manual', gated: false },
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
 * The phone client's tabs in display order: the shared operational tabs, then
 * the phone-only Accessibility tab, then the documentation tabs. Accessibility
 * sits with the operational settings (before the reference docs) because it is
 * a live control surface, not reference material.
 */
export function visibleClientTabs(demo) {
  return visibleTabs(demo)
    .concat(CLIENT_ACCESSIBILITY_TABS)
    .concat(CLIENT_DOCUMENTATION_TABS);
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

/** Resolve the selected tab against the phone client's complete tab list. */
export function resolveClientActiveTab(wanted, demo) {
  const tabs = visibleClientTabs(demo);
  if (tabs.some((tab) => tab.id === wanted)) return wanted;
  return tabs.length > 0 ? tabs[0].id : null;
}

/**
 * A Viewscreen's tabs in display order: the shared operational tabs, then the
 * endpoint's own Display tab (issue #1427).
 *
 * Display sits LAST among the ungated tabs and before Debug for the same reason
 * Accessibility sits after the operational tabs on the phone: it is the tab an
 * operator visits when setting the room up rather than during a session, so it
 * must not become the demo build's landing tab (`resolveActiveTab` falls back to
 * the first entry). The native surface filters this list again by what it can
 * actually put a control on — see `nativeSettingsView`.
 */
export function visibleViewscreenTabs(demo) {
  const shared = visibleTabs(demo);
  const debugAt = shared.findIndex((tab) => tab.id === 'debug');
  if (debugAt < 0) return shared.concat(VIEWSCREEN_PRESENTATION_TABS);
  return shared
    .slice(0, debugAt)
    .concat(VIEWSCREEN_PRESENTATION_TABS)
    .concat(shared.slice(debugAt));
}

/** Resolve the selected tab against a Viewscreen's complete tab list. */
export function resolveViewscreenActiveTab(wanted, demo) {
  const tabs = visibleViewscreenTabs(demo);
  if (tabs.some((tab) => tab.id === wanted)) return wanted;
  return tabs.length > 0 ? tabs[0].id : null;
}
