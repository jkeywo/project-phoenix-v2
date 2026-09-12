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
  // No authored widgets, which is what makes the widget region absent on the
  // default desk and on the fallback a removed preset resolves to (#1439).
  widgets: Object.freeze([]),
});

/** Panel DOM ids this build actually draws and safely owns the `hidden`
 * attribute of. `gm-station-controls` is deliberately excluded:
 * gm-station-puppet.js already owns its `hidden` state from puppet
 * availability, and a second writer here would race it. */
export const GM_ROLE_PRESET_PANEL_IDS = Object.freeze([
  'gm-map-panel', 'gm-inspector', 'gm-activity',
  'gm-comms-panel',
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

/** The four typed widgets a preset may compose (issue #1439). Closed, and the
 * same four `GM_WIDGET_TYPES` in `src/world/config.rs` refuses a fifth of at
 * world load: a card this build cannot draw is a card nobody authored. */
export const GM_WIDGET_TYPES = Object.freeze(['attention', 'workload', 'actions', 'note']);

/**
 * The GM action buttons an `actions` widget may repeat — the SAME list the
 * quick-action facet narrows, because they are the same buttons.
 *
 * A widget button never issues a `GmAction`: it activates the shipped control
 * by this DOM id, so the confirmation category, the admission check and the
 * feedback lifecycle are the ones that button already had. Rust validates an
 * authored id against `GM_WIDGET_ACTION_IDS`, which
 * `src/world/config_tests.rs` reads this file to keep in step.
 */
export const GM_WIDGET_ACTION_IDS = GM_ROLE_PRESET_QUICK_ACTION_IDS;

/** A `strings.csv` id and nothing else — the shape Rust already refused at
 * world load, re-checked here for a hand-poked payload's sake. An authored
 * note is text the String Table owns; it is rendered through `textContent`,
 * never parsed as markup. */
const isStringTableId = (value) => typeof value === 'string' && value.length > 0
  && /^[A-Za-z0-9._-]+$/.test(value);

function normaliseWidget(value) {
  if (!value || typeof value !== 'object') return undefined;
  const kind = value.type ?? value.kind;
  if (typeof value.id !== 'string' || value.id.length === 0) return undefined;
  if (!GM_WIDGET_TYPES.includes(kind)) return undefined;
  if (typeof value.label !== 'string' || value.label.length === 0) return undefined;
  const widget = { id: value.id, type: kind, label: value.label };
  if (kind === 'attention') {
    if (typeof value.band === 'string' && value.band) widget.band = value.band;
    if (typeof value.category === 'string' && value.category) widget.category = value.category;
  }
  if (kind === 'attention' || kind === 'workload') {
    if (typeof value.ship === 'string' && value.ship) widget.ship = value.ship;
  }
  if (kind === 'actions') {
    widget.actions = normaliseIdList(value.actions)
      .filter((id) => GM_WIDGET_ACTION_IDS.includes(id));
    if (widget.actions.length === 0) return undefined;
  }
  if (kind === 'note') {
    if (!isStringTableId(value.text)) return undefined;
    widget.text = value.text;
  }
  return widget;
}

/** Parse one preset's `[[gm_role_preset.widget]]` list. Same posture as the
 * preset list itself: a malformed or unknown-type entry is dropped rather than
 * drawn as an empty card, and a repeated id keeps the first — Rust refused
 * both at world load, so reaching either here means the payload was poked. */
function normaliseWidgets(value) {
  if (!Array.isArray(value)) return [];
  const seen = new Set();
  const out = [];
  for (const candidate of value) {
    const widget = normaliseWidget(candidate);
    if (!widget || seen.has(widget.id)) continue;
    seen.add(widget.id);
    out.push(widget);
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
    widgets: normaliseWidgets(value.widget ?? value.widgets),
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
  onEffective = () => {},
} = {}) {
  const selectEl = doc && doc.getElementById('gm-role-preset-select');
  let presets = [];
  let desiredId = null;
  let effective = GM_ALL_ROLE_PRESET;
  /** Why the effective preset last changed, for `onEffective`. See its
   * contract below: only `'select'` is the operator asking for this role. */
  let source = 'init';

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
    // The EFFECTIVE preset, every time it is resolved, with why. Issue #1439's
    // widget region is composed from this: the four typed cards follow the
    // preset in effect, whether it changed because the operator switched
    // (`'select'`), because a reconnect restored their last choice
    // (`'restore'`), or because a world (re)load removed the preset they were
    // on and it fell back to All (`'available'`).
    //
    // The distinction is load-bearing, not decorative. A widget's authored
    // default filters are applied on `'select'` alone — a live switch is the
    // operator asking for this role's view. On `'restore'` the private filters
    // and snoozes the operator actually left behind win, which is what makes
    // "same-session reconnect restores their state" true rather than "a
    // reconnect silently re-imposes the author's defaults over it".
    onEffective(effective, { source });
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
    source = 'available';
    reconcileEffective();
  }

  /** The operator's own live switch (or a programmatic equivalent). Always
   * notifies `onSelect`, even when the resolved/effective preset does not
   * change (e.g. re-selecting an id that was already falling back to All) —
   * the desired id is what gets persisted, and it may have changed even when
   * its effect has not. */
  function select(id) {
    desiredId = normaliseDesired(id);
    source = 'select';
    reconcileEffective();
    onSelect(desiredId);
  }

  /** Seed the desired id from a restored/reconnected identity WITHOUT
   * treating it as a fresh operator choice — this reads storage, it must
   * never write back to it (that would be the exact "restore stomps the
   * value it just restored" bug reconnect persistence has to avoid). */
  function restore(id) {
    desiredId = normaliseDesired(id);
    source = 'restore';
    reconcileEffective();
  }

  function state() {
    return {
      presets: [...presets],
      desiredId,
      effectivePresetId: effective.id,
      // The resolved preset itself, so a consumer that composes from it (the
      // #1439 widget region) can read what is in effect without re-resolving.
      effective,
    };
  }

  if (selectEl) {
    selectEl.addEventListener('change', () => select(selectEl.value));
  }

  paintOptions();
  reconcileEffective();

  return { setAvailablePresets, select, restore, state };
}
