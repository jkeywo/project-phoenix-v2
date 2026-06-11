/**
 * gui/complexity-ui.js — Pure logic for the lobby complexity preset selector
 * in client.html. Issue #461: ports refresh_complexity_ui /
 * handle_complexity_preset_press from the deleted Bevy systems, generalised
 * from hard-coded Tactical to any console the local player holds. (The Bevy
 * first-use pop-up was dropped — the always-visible lobby segmented selector
 * supersedes it.)
 *
 * All functions take the ComplexityStore as an explicit argument so they are
 * unit-testable in Node (client.html passes window.complexityStore).
 *
 * Exposed on `window` as `window.complexityUi`.
 */

import { setComplexityMessage } from './complexity-store.js';

/**
 * Materialise store choices for every console the local player holds.
 * Needed so `ComplexityStore.apply()` picks up own-console ComplexityChanged
 * broadcasts (apply only updates already-materialised choices — see #460).
 *
 * @param {import('./complexity-store.js').ComplexityStore} store
 * @param {string[]} consoles Consoles held by the local player.
 */
export function materialiseConsoles(store, consoles) {
  for (const c of consoles || []) store.forConsole(c);
}

/**
 * State for the segmented preset selector of one console.
 *
 * @param {import('./complexity-store.js').ComplexityStore} store
 * @param {string} consoleName
 * @returns {{ presets: string[], active: string, showDropdown: boolean }}
 */
export function dropdownState(store, consoleName) {
  const choice = store.forConsole(consoleName);
  return {
    presets: [...choice.availablePresets],
    active: choice.effectivePreset(),
    showDropdown: choice.showDropdown(),
  };
}

/**
 * Select a preset from the segmented control: persist via the store and
 * return the SetComplexity ClientMessage to send (null when invalid).
 *
 * @param {import('./complexity-store.js').ComplexityStore} store
 * @param {string} consoleName
 * @param {string} presetName
 * @returns {{ type: string, data: object } | null}
 */
export function selectPreset(store, consoleName, presetName) {
  if (!store.select(consoleName, presetName)) return null;
  return setComplexityMessage(consoleName, presetName);
}

if (typeof window !== 'undefined') {
  window.complexityUi = Object.freeze({
    materialiseConsoles,
    dropdownState,
    selectPreset,
  });
}
