/**
 * gui/host-scenarios.js — the pure view-model behind server.html's
 * scenario-catalog picker and applied-mod-pack list (issue #1230).
 *
 * server.html's `renderScenarioLockState()` and `renderModPackList()` are DOM
 * writers closed over host state (`_scenarioCatalog`, `_preSelection`, plus
 * two `wasmBindings` reads) — there is no way to unit-test the DECISIONS they
 * make without booting a real WASM binding and a document. This module lifts
 * those decisions out: given the same catalog/selection/report data
 * server.html already holds, it returns what to show, and server.html keeps
 * every DOM write, the `wasm_active_pack_manifest()`/`wasm_reorder_mod_packs()`
 * calls, and the auto-select/broadcast side effects that follow a render.
 *
 * Both functions are siblings of gui/scenario-arbiter.js (issue #755, first-
 * valid-wins selection) rather than a merge into it: the arbiter decides
 * WHETHER a selection request is accepted; this module decides what the
 * ALREADY-DECIDED state should render as. `scenarioCatalogView` reuses the
 * arbiter's own `findScenario` rather than re-walking the catalog by hand, so
 * the "which catalog entry does this scenario_id name" lookup has exactly one
 * implementation.
 */

import { findScenario, normalizeSelection } from './scenario-arbiter.js';

/**
 * The pure decision behind `renderScenarioLockState()`: which stage of the
 * QR-first picker to show, or that the picker is locked out entirely.
 *
 * `locked` is server.html's own `_worldLoadStarted` flag, passed in rather
 * than re-derived from `preSelection` — every OTHER guard beside this render
 * in server.html (`arbiterSelectScenario`, the mod-pack upload handler, the
 * save-import handler) checks `_worldLoadStarted` directly, and at least one
 * caller (the `?scenario=<path>` dev bypass, and the save-import bypass) sets
 * it — or its `selectedScenario` equivalent — WITHOUT ever completing
 * `_preSelection`. Deriving "locked" from selection-completeness alone would
 * make this function silently wrong the day a bypass like that starts calling
 * it; taking the flag as an explicit input keeps the two concerns separate,
 * matching how the surrounding glue already treats them.
 *
 * @param {Array<{id: string, world: string, label: *, ships?: Array<{template_path: string, label?: *}>}>} catalog
 *   the #754 pre-load catalog (`_scenarioCatalog` / `wasm_get_scenario_catalog()`).
 * @param {{scenario_id: string|null, template_path: string|null}|null} preSelection
 *   the arbiter's current lock state (`_preSelection`).
 * @param {boolean} locked world load has already started — render nothing.
 * @returns {
 *   | {stage: 'locked'}
 *   | {stage: 'scenario-empty', labelId: string}
 *   | {stage: 'scenario-list', labelId: string, entries: Array<{scenarioId: string, world: string, label: *}>}
 *   | {stage: 'ship-auto', templatePath: string}
 *   | {stage: 'ship-picker', labelId: string, ships: Array<{template_path: string, label?: *}>}
 * } what server.html should render. 'ship-auto' asks the caller to invoke its
 *   own `arbiterSelectShip(templatePath)` (a side effect this module cannot
 *   perform) rather than render anything — mirrors the original inline
 *   single-ship auto-resolve (issue #917).
 */
export function scenarioCatalogView(catalog, preSelection, locked) {
  const list = Array.isArray(catalog) ? catalog : [];
  const sel = normalizeSelection(preSelection);

  if (locked) return { stage: 'locked' };

  if (sel.scenario_id == null) {
    if (list.length === 0) {
      return { stage: 'scenario-empty', labelId: 'server.select_world' };
    }
    return {
      stage: 'scenario-list',
      labelId: 'server.select_world',
      entries: list.map((sc) => ({ scenarioId: sc.id, world: sc.world, label: sc.label })),
    };
  }

  if (sel.template_path == null) {
    const entry = findScenario(list, sel.scenario_id);
    const ships = (entry && entry.ships) || [];
    // A scenario curated (or authored) down to exactly one playable hull
    // resolves straight to it — no picker click needed (issue #917).
    // Count-based, not keyed on any hull name.
    if (ships.length === 1) {
      return { stage: 'ship-auto', templatePath: ships[0].template_path };
    }
    return { stage: 'ship-picker', labelId: 'server.select_ship', ships };
  }

  // Both locked — world load is about to start (driveWorldLoad requires
  // isComplete(_preSelection)); same "clear and add nothing" render as the
  // explicit `locked` flag above.
  return { stage: 'locked' };
}

/**
 * The pure row/conflict data behind `renderModPackList()`.
 *
 * @param {{
 *   packs?: Array<{id: string, name?: string, version?: string, file_count?: number}>,
 *   conflicts?: Array<{path: string, winner: string, losers?: string[]}>,
 * }|null|undefined} report `wasm_active_pack_manifest()`'s return value — the
 *   SAME call both the host list and the phone's `active_packs` wire payload
 *   read, so host and phones never derive two different answers.
 * @returns {{
 *   visible: boolean,
 *   packs: Array<{id: string, name: string, version: string, fileCount: number, canMoveUp: boolean, canMoveDown: boolean}>,
 *   conflicts: Array<{path: string, winner: string, losers: string[]}>,
 * }} `visible: false` (with empty arrays) when there is nothing to show —
 *   `renderModPackList()`'s early return on a zero-pack report.
 */
export function modPackListView(report) {
  const packs = report && report.packs ? Array.from(report.packs) : [];
  if (packs.length === 0) return { visible: false, packs: [], conflicts: [] };

  const packRows = packs.map((pack, idx) => ({
    id: pack.id,
    name: pack.name || pack.id || '',
    version: pack.version || '',
    fileCount: pack.file_count != null ? pack.file_count : 0,
    canMoveUp: idx > 0,
    canMoveDown: idx < packs.length - 1,
  }));

  const conflicts = report && report.conflicts ? Array.from(report.conflicts) : [];
  const conflictRows = conflicts.map((c) => ({
    path: c.path,
    winner: c.winner,
    losers: c.losers ? Array.from(c.losers) : [],
  }));

  return { visible: true, packs: packRows, conflicts: conflictRows };
}

// Expose for the classic-script consumer (server.html is not a module).
if (typeof window !== 'undefined') {
  window.hostScenarios = { scenarioCatalogView, modPackListView };
}
