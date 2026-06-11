/**
 * gui/complexity-ui.js — Pure logic for the complexity preset shell UI
 * (first-use pop-up + lobby segmented selector) in client.html.
 * Issue #461: ports refresh_complexity_ui / handle_complexity_preset_press /
 * handle_complexity_popup_confirm from the deleted Bevy systems, generalised
 * from hard-coded Tactical to any console the local player holds.
 *
 * All functions take the ComplexityStore as an explicit argument so they are
 * unit-testable in Node (client.html passes window.complexityStore).
 *
 * Exposed on `window` as `window.complexityUi`.
 */

import { setComplexityMessage } from './complexity-store.js';

/** Default selection on the first-use pop-up (mirrors the Bevy confirm fallback). */
export const POPUP_DEFAULT_PRESET = 'Low';

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
 * Decide whether a first-use pop-up should be shown, and for which console.
 * Returns the first held console whose choice wants a pop-up (multi-preset,
 * nothing chosen, not yet shown), or null when no pop-up is due.
 *
 * @param {import('./complexity-store.js').ComplexityStore} store
 * @param {string[]} consoles Consoles held by the local player.
 * @returns {{ console: string, presets: string[], defaultPreset: string } | null}
 */
export function popupPlan(store, consoles) {
  for (const c of consoles || []) {
    const choice = store.forConsole(c);
    if (choice.showPopup()) {
      return {
        console: c,
        presets: [...choice.availablePresets],
        defaultPreset: POPUP_DEFAULT_PRESET,
      };
    }
  }
  return null;
}

/**
 * Confirm the pop-up: select the preset (falling back to the Low default
 * when none was tapped, mirroring handle_complexity_popup_confirm), persist
 * via the store, and return the SetComplexity ClientMessage to send.
 *
 * @param {import('./complexity-store.js').ComplexityStore} store
 * @param {string} consoleName
 * @param {string|null} [presetName] The tapped preset, or null for the default.
 * @returns {{ type: string, data: object } | null} Message to send, or null
 *          when the selection was invalid for this console.
 */
export function confirmPopup(store, consoleName, presetName) {
  const name = presetName || POPUP_DEFAULT_PRESET;
  if (!store.select(consoleName, name)) return null;
  return setComplexityMessage(consoleName, name);
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
    POPUP_DEFAULT_PRESET,
    materialiseConsoles,
    popupPlan,
    confirmPopup,
    dropdownState,
    selectPreset,
  });
}
