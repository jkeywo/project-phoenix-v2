/**
 * gui/scenario-arbiter.js — Pure host-runtime arbiter for the QR-first
 * pre-scenario selection flow (issue #755).
 *
 * The host page (server.html) is the authority. Before any world is loaded it
 * accepts scenario + player-ship selection requests from BOTH its own UI and
 * connected phones, applying a single authoritative rule:
 *
 *   first valid request wins.
 *
 * There is no voting and no pre-ship captain authority — whichever participant
 * sends the first request that validates against the pre-load catalog locks the
 * choice; every later request is ignored. Selections are validated against the
 * #754 catalog (scenario id must exist; ship template must be one the *locked
 * scenario* offers — this scopes ships to their scenario, #754 AC4).
 *
 * All functions are pure (no DOM, no transport) so server.html can drive both
 * transports through them and Vitest can exercise the rules directly.
 *
 * A "selection" is `{ scenario_id, slot_id, template_path }`.
 * Each entry point returns `{ outcome, selection }` where `outcome` is one of
 * 'accepted' (the request locked a new value), 'ignored' (already locked), or
 * 'rejected' (failed catalog validation). `selection` is always a fresh object
 * — callers replace their held state with it on 'accepted'.
 */

/** Normalise a possibly-partial selection into the canonical shape. */
export function normalizeSelection(sel) {
  const normalized = {
    scenario_id: (sel && sel.scenario_id) || null,
    template_path: (sel && sel.template_path) || null,
  };
  // Preserve the historical two-field shape for legacy callers and the
  // browser/native parity fixture. The slot key becomes part of the selection
  // only after a slot-aware catalogue/caller introduces it.
  if (sel && Object.prototype.hasOwnProperty.call(sel, 'slot_id')) {
    normalized.slot_id = sel.slot_id || null;
  }
  return normalized;
}

/** Find a catalog entry by scenario id, or null. */
export function findScenario(catalog, scenarioId) {
  if (!Array.isArray(catalog)) return null;
  return catalog.find((s) => s && s.id === scenarioId) || null;
}

/**
 * Apply a scenario selection request. First-valid-wins: once a scenario is
 * locked, further requests are ignored; an unknown scenario id is rejected.
 */
export function selectScenario(selection, catalog, scenarioId) {
  const sel = normalizeSelection(selection);
  if (sel.scenario_id != null) return { outcome: 'ignored', selection: sel };
  if (!findScenario(catalog, scenarioId)) return { outcome: 'rejected', selection: sel };
  return {
    outcome: 'accepted',
    selection: { scenario_id: scenarioId, template_path: null },
  };
}

/** Claim one authored mission slot. The caller supplies claimant identity. */
export function selectShipSlot(selection, catalog, slotId, claimant, reservations) {
  const sel = normalizeSelection(selection);
  if (sel.scenario_id == null) return { outcome: 'rejected', selection: sel };
  if (sel.slot_id != null) return { outcome: 'ignored', selection: sel };
  const entry = findScenario(catalog, sel.scenario_id);
  const slots = (entry && Array.isArray(entry.slots)) ? entry.slots : [];
  if (!slots.some(slot => slot && slot.id === slotId)) return { outcome: 'rejected', selection: sel };
  const held = reservations && reservations[slotId];
  if (held && held.claimant !== claimant) return { outcome: 'occupied', selection: sel };
  if (reservations) reservations[slotId] = { claimant, hull: null };
  return { outcome: 'accepted', selection: { scenario_id: sel.scenario_id, slot_id: slotId, template_path: null } };
}

export function releaseShipSlot(selection, claimant, reservations) {
  const sel = normalizeSelection(selection);
  if (sel.slot_id == null) return { outcome: 'ignored', selection: sel };
  const held = reservations && reservations[sel.slot_id];
  if (!held || held.claimant !== claimant) return { outcome: 'rejected', selection: sel };
  delete reservations[sel.slot_id];
  return { outcome: 'accepted', selection: { scenario_id: sel.scenario_id, slot_id: null, template_path: null } };
}

/**
 * Apply a player-ship selection request. Requires a scenario to already be
 * locked (ships are scoped to their scenario); the template must be one the
 * locked scenario offers. First-valid-wins on the ship as well.
 */
export function selectPlayerShip(selection, catalog, templatePath, claimant, reservations) {
  const sel = normalizeSelection(selection);
  if (sel.scenario_id == null) return { outcome: 'rejected', selection: sel };
  if (sel.template_path != null) return { outcome: 'ignored', selection: sel };
  const entry = findScenario(catalog, sel.scenario_id);
  if (!entry) return { outcome: 'rejected', selection: sel };
  const slots = Array.isArray(entry.slots) ? entry.slots : [];
  const slot = slots.find(row => row && row.id === sel.slot_id);
  if (slots.length > 0 && !slot) return { outcome: 'rejected', selection: sel };
  if (slot) {
    const held = reservations && reservations[slot.id];
    if (!held || held.claimant !== claimant) return { outcome: 'rejected', selection: sel };
  }
  const offeredShips = slot ? slot.ships : entry.ships;
  const offered =
    Array.isArray(offeredShips) &&
    offeredShips.some((sh) => sh && sh.template_path === templatePath);
  if (!offered) return { outcome: 'rejected', selection: sel };
  if (slot && reservations) reservations[slot.id].hull = templatePath;
  return {
    outcome: 'accepted',
    selection: sel.slot_id != null
      ? { scenario_id: sel.scenario_id, slot_id: sel.slot_id, template_path: templatePath }
      : { scenario_id: sel.scenario_id, template_path: templatePath },
  };
}

/** True once both a scenario and a ship are locked — ready to load the world. */
export function isComplete(selection, catalog) {
  const sel = normalizeSelection(selection);
  const entry = findScenario(catalog, sel.scenario_id);
  const needsSlot = !!(entry && Array.isArray(entry.slots) && entry.slots.length);
  return sel.scenario_id != null && (!needsSlot || sel.slot_id != null) && sel.template_path != null;
}

/** Resolve the world TOML path for the locked scenario (null if unknown). */
export function worldPathFor(catalog, selection) {
  const sel = normalizeSelection(selection);
  const entry = findScenario(catalog, sel.scenario_id);
  return entry ? entry.world : null;
}

/**
 * Template paths the locked scenario's catalog entry offers (issue #917).
 *
 * The catalog entry's `ships` list is already curation-filtered by the Rust
 * side (`world::manifest::build_catalog`) — non-empty only when the manifest
 * curated the world down to a subset, otherwise it's every ship the world
 * offers. Either way this is exactly the allowlist `wasm_load_world` needs to
 * restrict which hulls get preloaded: empty (unresolved selection, or a world
 * with no `[[available_ships]]`) means unrestricted, matching
 * `ScenarioEntry.ships`'s own semantics. Count-based, not keyed on any hull
 * name.
 */
export function curatedShipsFor(catalog, selection) {
  const sel = normalizeSelection(selection);
  const entry = findScenario(catalog, sel.scenario_id);
  const ships = (entry && entry.ships) || [];
  return ships.map((s) => s.template_path);
}

// Expose for classic-script consumers (server.html is not a module).
if (typeof window !== 'undefined') {
  window.scenarioArbiter = {
    normalizeSelection,
    findScenario,
    selectScenario,
    selectShipSlot,
    releaseShipSlot,
    selectPlayerShip,
    isComplete,
    worldPathFor,
    curatedShipsFor,
  };
}
