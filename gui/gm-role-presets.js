/**
 * gui/gm-role-presets.js — Presentation-only GM role presets (issue #1319,
 * PASM `gm-t2-performing-surface`).
 *
 * A role preset is scenario-authored personal presentation: a Game Master
 * chooses (and may live-switch) one, and it narrows which panels, quick
 * actions and contacts THEIR OWN browser shows. It defaults to the built-in
 * All preset, permits duplicate selection across independent GMs, and
 * persists only with that Game Master's own reconnectable identity
 * (server.html's `rememberGmIdentity`/`storedGmIdentity`).
 *
 * It is deliberately never authority: nothing here reaches `GmOperator`, the
 * crew-public GM roster, a `GmAction`, a snapshot, or the sim digest — every
 * Game Master keeps identical action availability, permissions, locks,
 * ordering and authority regardless of what preset they or any other GM has
 * selected. See `pasm/spec/design/gm-console-t2.yaml`.
 *
 * `panels`/`quickActions`/`contacts` are open string-id vocabularies, not a
 * closed enum, so a preset may already name a panel this build does not draw
 * yet — the M6-compatible descriptor shape #1317's Comms Studio and #1318's
 * knowledge comparison extend without a schema change.
 */

export const GM_ALL_ROLE_PRESET_ID = 'all';

/**
 * The one preset that always exists, is never authored (world TOML refuses
 * the id `"all"` — see `src/world/config.rs`'s `GM_ROLE_PRESET_ALL_ID`), and
 * every fallback resolves to. Empty facet lists mean "unrestricted" for every
 * facet a consuming panel checks.
 */
export const GM_ALL_ROLE_PRESET = Object.freeze({
  id: GM_ALL_ROLE_PRESET_ID,
  label: 'server.gm.role_preset.option.all',
  panels: Object.freeze([]),
  quickActions: Object.freeze([]),
  contacts: Object.freeze([]),
});

/** Panel DOM ids this build actually draws and safely owns the `hidden`
 * attribute of. `gm-station-controls` is deliberately excluded:
 * gm-station-puppet.js already owns its `hidden` state from puppet
 * availability, and a second writer here would race it. */
export const GM_ROLE_PRESET_PANEL_IDS = Object.freeze([
  'gm-map-panel', 'gm-inspector', 'gm-activity',
]);

/** Quick-action DOM ids this build actually draws. */
export const GM_ROLE_PRESET_QUICK_ACTION_IDS = Object.freeze([
  'gm-session-pause', 'gm-session-resume',
]);

function normaliseIdList(value) {
  if (!Array.isArray(value)) return [];
  const out = [];
  for (const entry of value) {
    if (typeof entry === 'string' && entry.length > 0 && !out.includes(entry)) out.push(entry);
  }
  return out;
}

function normalisePreset(value) {
  if (!value || typeof value !== 'object'
      || typeof value.id !== 'string' || value.id.length === 0
      || value.id === GM_ALL_ROLE_PRESET_ID) return undefined;
  return {
    id: value.id,
    label: typeof value.label === 'string' ? value.label : '',
    panels: normaliseIdList(value.panels),
    quickActions: normaliseIdList(value.quick_actions ?? value.quickActions),
    contacts: normaliseIdList(value.contacts),
  };
}

/**
 * Parse the scenario-authored preset list carried by
 * `wasm_get_gm_role_presets()` — a JSON string, or an already-parsed array.
 *
 * Malformed entries are dropped rather than failing the whole list; the
 * reserved `'all'` id and a repeated id are dropped too (Rust already refuses
 * both at world-parse time — this is defense in depth for a hand-poked
 * payload, the same posture as `gm-activity-feed.js`'s `normaliseEntry`).
 * Unparseable or non-array input yields an empty list: the safe fallback is
 * always just the built-in All preset (acceptance criterion 1).
 */
export function parseGmRolePresets(payload) {
  let value = payload;
  if (typeof value === 'string') {
    try { value = JSON.parse(value); } catch (_) { return []; }
  }
  if (!Array.isArray(value)) return [];
  const seen = new Set();
  const out = [];
  for (const candidate of value) {
    const preset = normalisePreset(candidate);
    if (!preset || seen.has(preset.id)) continue;
    seen.add(preset.id);
    out.push(preset);
  }
  return out;
}

/**
 * Resolve the preset a Game Master's desired id names, against the CURRENT
 * authored list. A missing, invalid, or removed id — including the reserved
 * `'all'` and a null/undefined choice — falls back to the built-in All
 * rather than refusing or leaving a stale selection live. Two operators may
 * pass the same or different ids independently; this is a pure lookup with
 * no shared state, so nothing about one call can affect another.
 */
export function resolveGmRolePreset(presets, id) {
  if (typeof id === 'string' && id !== GM_ALL_ROLE_PRESET_ID) {
    const found = (presets || []).find((preset) => preset.id === id);
    if (found) return found;
  }
  return GM_ALL_ROLE_PRESET;
}

const unrestricted = (list) => !Array.isArray(list) || list.length === 0;

/** Is `panelId` shown under `preset`? Any string id — ids this build draws
 * today and ids a future milestone adds alike. */
export function isGmPanelVisible(preset, panelId) {
  return unrestricted(preset && preset.panels) || preset.panels.includes(panelId);
}

/** Is `actionId` shown under `preset`? Hiding a quick action is presentation
 * only: the underlying `GmAction` stays fully admissible for every operator
 * regardless of any preset, on any browser, at any time. */
export function isGmQuickActionVisible(preset, actionId) {
  return unrestricted(preset && preset.quickActions) || preset.quickActions.includes(actionId);
}

/** Is `contactId` (a world-entity name) shown under `preset`? No shipped
 * panel reads this yet — the map/inspector/activity feed stay omniscient —
 * it exists so #1317/#1318's contact-scoped panels have a ready vocabulary. */
export function isGmContactVisible(preset, contactId) {
  return unrestricted(preset && preset.contacts) || preset.contacts.includes(contactId);
}

/**
 * DOM controller for the GM console's role-preset selector.
 *
 * `onSelect(desiredId)` fires only when the OPERATOR's own explicit choice
 * changes (via `select()`), never when the available list changes underneath
 * an unresolvable choice — that is exactly what should be persisted, and
 * exactly what a later `setAvailablePresets` call (e.g. a preset reappearing
 * after a reload) should re-resolve fresh rather than have already
 * overwritten. `desiredId` is `null` for the built-in All.
 */
export function createGmRolePresets({
  doc = globalThis.document,
  t = (id) => id,
  onSelect = () => {},
} = {}) {
  const selectEl = doc && doc.getElementById('gm-role-preset-select');
  let presets = [];
  let desiredId = null;
  let effective = GM_ALL_ROLE_PRESET;

  function paintOptions() {
    if (!selectEl) return;
    selectEl.replaceChildren();
    const allOption = doc.createElement('option');
    allOption.value = GM_ALL_ROLE_PRESET_ID;
    allOption.textContent = t(GM_ALL_ROLE_PRESET.label);
    selectEl.appendChild(allOption);
    for (const preset of presets) {
      const option = doc.createElement('option');
      option.value = preset.id;
      option.textContent = preset.label ? t(preset.label) : preset.id;
      selectEl.appendChild(option);
    }
  }

  function applyVisibility() {
    for (const id of GM_ROLE_PRESET_PANEL_IDS) {
      const el = doc && doc.getElementById(id);
      if (el) el.hidden = !isGmPanelVisible(effective, id);
    }
    for (const id of GM_ROLE_PRESET_QUICK_ACTION_IDS) {
      const el = doc && doc.getElementById(id);
      if (el) el.hidden = !isGmQuickActionVisible(effective, id);
    }
  }

  function reconcileEffective() {
    effective = resolveGmRolePreset(presets, desiredId);
    if (selectEl) {
      selectEl.value = presets.some((preset) => preset.id === desiredId)
        ? desiredId
        : GM_ALL_ROLE_PRESET_ID;
    }
    applyVisibility();
  }

  function normaliseDesired(id) {
    return (typeof id === 'string' && id.length > 0 && id !== GM_ALL_ROLE_PRESET_ID) ? id : null;
  }

  /** The scenario-authored list changed (world (re)load, mod-pack switch).
   * Re-resolves the CURRENT desired id against it without touching that id —
   * a preset that disappears falls back to All in effect only; a preset that
   * reappears is honoured again without the operator re-selecting it. */
  function setAvailablePresets(rawPayload) {
    presets = parseGmRolePresets(rawPayload);
    paintOptions();
    reconcileEffective();
  }

  /** The operator's own live switch (or a programmatic equivalent). Always
   * notifies `onSelect`, even when the resolved/effective preset does not
   * change (e.g. re-selecting an id that was already falling back to All) —
   * the desired id is what gets persisted, and it may have changed even when
   * its effect has not. */
  function select(id) {
    desiredId = normaliseDesired(id);
    reconcileEffective();
    onSelect(desiredId);
  }

  /** Seed the desired id from a restored/reconnected identity WITHOUT
   * treating it as a fresh operator choice — this reads storage, it must
   * never write back to it (that would be the exact "restore stomps the
   * value it just restored" bug reconnect persistence has to avoid). */
  function restore(id) {
    desiredId = normaliseDesired(id);
    reconcileEffective();
  }

  function state() {
    return { presets: [...presets], desiredId, effectivePresetId: effective.id };
  }

  if (selectEl) {
    selectEl.addEventListener('change', () => select(selectEl.value));
  }

  paintOptions();
  reconcileEffective();

  return { setAvailablePresets, select, restore, state };
}
