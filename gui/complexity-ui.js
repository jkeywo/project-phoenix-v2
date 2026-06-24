/**
 * gui/complexity-ui.js - Pure logic for the lobby complexity preset selector
 * in client.html.
 */

import { setComplexityMessage } from './complexity-store.js';

export function materialiseConsoles(store, consoles) {
  for (const c of consoles || []) store.forConsole(c);
}

export function dropdownState(store, consoleName) {
  const choice = store.forConsole(consoleName);
  return {
    presets: [...choice.availablePresets],
    active: choice.effectivePreset(),
    showDropdown: choice.showDropdown(),
  };
}

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
