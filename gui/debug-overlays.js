/**
 * gui/debug-overlays.js — the debug dock's four migrated legacy overlays
 * (issue #1150, PRD #1144).
 *
 * The JSON-driven renderers that replaced the pre-formatted TEXT streams the
 * modifier / damage / entity-behavior / entity-inspector overlays used to paint
 * as `textContent`. Each parses the structured payload its Rust surface now
 * publishes (`wasm_get_debug_state`, `wasm_get_damage_log`,
 * `wasm_get_entity_debug_state`, `wasm_get_entity_inspector`) and builds DOM the
 * settings cog drops into the dock — the same renderer pattern
 * `gui/station-activity-chart.js` established: a `parse*` guard, a pure
 * `build*(payload)` returning detached DOM, and a `render*(container, json)`
 * wrapper the cog wires in. Pure functions of the payload, so they are
 * unit-tested in jsdom without a browser or a WASM bundle.
 */

import { t } from './strings.js';

/**
 * Parse a raw bridge JSON string into an object, or `null` when there is nothing
 * renderable yet (empty string before the first publish, or malformed input).
 *
 * @param {string} json
 * @returns {object|null}
 */
export function parseDebugPayload(json) {
  if (typeof json !== 'string' || json.length === 0) return null;
  try {
    const payload = JSON.parse(json);
    return payload && typeof payload === 'object' ? payload : null;
  } catch {
    return null;
  }
}

/** Append a titled section wrapper and return its body element. */
function section(root, doc, titleText) {
  const sec = doc.createElement('div');
  sec.className = 'dbg-section';
  const h = doc.createElement('div');
  h.className = 'dbg-section-title';
  h.textContent = titleText;
  sec.appendChild(h);
  root.appendChild(sec);
  return sec;
}

/** A `(none)` / empty-state line inside a section body. */
function emptyLine(doc, text) {
  const el = doc.createElement('div');
  el.className = 'dbg-empty';
  el.textContent = text;
  return el;
}

function fmt(n, digits = 1) {
  return Number.isFinite(n) ? n.toFixed(digits) : '—';
}

// ── Modifier surface ─────────────────────────────────────────────────────────

/**
 * Build the modifier overlay DOM from a parsed `ModifierDebugPayload`. Pure.
 * @param {object} payload
 * @param {{doc?: Document}} [opts]
 * @returns {HTMLElement}
 */
export function buildModifierDebug(payload, opts = {}) {
  const doc = opts.doc || document;
  const root = doc.createElement('div');
  root.className = 'dbg-modifiers';

  const flags = section(root, doc, t('settings.debug.modifiers'));
  const flagList = Array.isArray(payload.flags) ? payload.flags : [];
  if (flagList.length === 0) {
    flags.appendChild(emptyLine(doc, '(none)'));
  } else {
    for (const f of flagList) {
      const row = doc.createElement('div');
      row.className = 'dbg-row';
      row.setAttribute('data-flag', f.flag);
      row.textContent = `${f.flag} ← ${(f.sources || []).join(', ')}`;
      flags.appendChild(row);
    }
  }

  const floats = section(root, doc, 'Float Modifiers');
  const floatList = Array.isArray(payload.float_modifiers) ? payload.float_modifiers : [];
  if (floatList.length === 0) {
    floats.appendChild(emptyLine(doc, '(none)'));
  } else {
    for (const m of floatList) {
      const row = doc.createElement('div');
      row.className = 'dbg-row';
      row.setAttribute('data-slot', m.slot);
      const detail = (m.contributions || [])
        .map((c) => `${c.source} (${c.bonus >= 0 ? '+' : ''}${fmt(c.bonus, 2)})`)
        .join(', ');
      row.textContent = `${m.slot} ×${fmt(m.multiplier, 2)} ← ${detail}`;
      floats.appendChild(row);
    }
  }

  const ints = section(root, doc, 'Int Modifiers');
  const intList = Array.isArray(payload.int_modifiers) ? payload.int_modifiers : [];
  if (intList.length === 0) {
    ints.appendChild(emptyLine(doc, '(none)'));
  } else {
    for (const m of intList) {
      const row = doc.createElement('div');
      row.className = 'dbg-row';
      row.setAttribute('data-slot', m.slot);
      const detail = (m.contributions || [])
        .map((c) => `${c.source} (${c.bonus >= 0 ? '+' : ''}${c.bonus})`)
        .join(', ');
      row.textContent = `${m.slot} ${m.sum >= 0 ? '+' : ''}${m.sum} ← ${detail}`;
      ints.appendChild(row);
    }
  }

  return root;
}

// ── Damage surface ───────────────────────────────────────────────────────────

/**
 * Build the damage overlay DOM from a parsed `DamageDebugPayload`. Pure.
 * @param {object} payload
 * @param {{doc?: Document}} [opts]
 * @returns {HTMLElement}
 */
export function buildDamageDebug(payload, opts = {}) {
  const doc = opts.doc || document;
  const root = doc.createElement('div');
  root.className = 'dbg-damage';
  const entries = Array.isArray(payload.entries) ? payload.entries : [];

  const title = doc.createElement('div');
  title.className = 'dbg-section-title';
  title.textContent = t('settings.debug.damage');
  root.appendChild(title);

  if (entries.length === 0) {
    root.appendChild(emptyLine(doc, '(no damage)'));
    return root;
  }

  entries.forEach((e, i) => {
    const row = doc.createElement('div');
    row.className = 'dbg-row';
    row.setAttribute('data-source', e.source);
    const arc = e.shield_arc == null ? '—' : e.shield_arc;
    row.textContent = `${i + 1}. ${e.source}  arc=${arc}  dmg=${fmt(e.amount, 1)}`;
    root.appendChild(row);
  });

  return root;
}

// ── Entity-behavior surface ──────────────────────────────────────────────────

/**
 * Build the entity-behavior overlay DOM from a parsed `EntityBehaviorPayload`.
 * Pure.
 * @param {object} payload
 * @param {{doc?: Document}} [opts]
 * @returns {HTMLElement}
 */
export function buildEntityBehaviorDebug(payload, opts = {}) {
  const doc = opts.doc || document;
  const root = doc.createElement('div');
  root.className = 'dbg-entities';
  const entries = Array.isArray(payload.entries) ? payload.entries : [];

  const title = doc.createElement('div');
  title.className = 'dbg-section-title';
  title.textContent = `${t('settings.debug.entities')} (${entries.length})`;
  root.appendChild(title);

  if (entries.length === 0) {
    root.appendChild(emptyLine(doc, '(none)'));
    return root;
  }

  entries.forEach((e, i) => {
    const row = doc.createElement('div');
    row.className = 'dbg-row';
    row.setAttribute('data-name', e.name);
    row.textContent = `${i + 1}. ${e.name}  pos=(${fmt(e.x)}, ${fmt(e.y)}, ${fmt(e.z)})  target=${e.target}`;
    root.appendChild(row);
  });

  return root;
}

// ── Entity-inspector surface ─────────────────────────────────────────────────

/**
 * Build the entity-inspector overlay DOM from a parsed `EntityInspectorPayload`.
 * Pure.
 * @param {object} payload
 * @param {{doc?: Document}} [opts]
 * @returns {HTMLElement}
 */
export function buildEntityInspectorDebug(payload, opts = {}) {
  const doc = opts.doc || document;
  const root = doc.createElement('div');
  root.className = 'dbg-inspector';

  const title = doc.createElement('div');
  title.className = 'dbg-section-title';
  title.textContent = t('settings.debug.inspector');
  root.appendChild(title);

  if (payload.player) {
    const p = payload.player;
    const block = doc.createElement('div');
    block.className = 'dbg-player';
    const head = doc.createElement('div');
    head.className = 'dbg-row';
    head.textContent = `[Player Ship]  pos=(${fmt(p.x)}, ${fmt(p.z)})`;
    block.appendChild(head);

    const hull = doc.createElement('div');
    hull.className = 'dbg-row';
    hull.setAttribute('data-field', 'hull');
    const hullEntries = Array.isArray(p.hull) ? p.hull : [];
    hull.textContent = hullEntries.length
      ? `  hull:${hullEntries.map((h) => `  ${h.system} ${Math.round(h.current)}/${Math.round(h.max)}`).join('')}`
      : '  hull: n/a';
    block.appendChild(hull);

    const shields = doc.createElement('div');
    shields.className = 'dbg-row';
    shields.setAttribute('data-field', 'shields');
    const facings = Array.isArray(p.shields) ? p.shields : [];
    shields.textContent = facings.length
      ? `  shields:${facings
          .map((f) => {
            const pct = f.max_hp > 0 ? Math.round((f.hp / f.max_hp) * 100) : 0;
            const focus = f.focused ? '*' : '';
            const off = f.offline ? ' [OFFLINE]' : '';
            return `  ${focus}${f.label} ${f.hp}/${f.max_hp} (${pct}%)${off}`;
          })
          .join('')}`
      : '  shields: n/a';
    block.appendChild(shields);

    root.appendChild(block);
  }

  const entities = Array.isArray(payload.entities) ? payload.entities : [];
  entities.forEach((e, i) => {
    const block = doc.createElement('div');
    block.className = 'dbg-entity';
    block.setAttribute('data-name', e.name);

    const head = doc.createElement('div');
    head.className = 'dbg-row';
    head.textContent = `${i + 1}. ${e.name}  [${(e.tags || []).join(', ')}]`;
    block.appendChild(head);

    const pos = doc.createElement('div');
    pos.className = 'dbg-row';
    pos.textContent = `    pos=(${fmt(e.x)}, ${fmt(e.z)})  dist=${fmt(e.distance)}u`;
    block.appendChild(pos);

    if (e.faction != null) {
      const line = doc.createElement('div');
      line.className = 'dbg-row';
      line.textContent = `    faction: ${e.faction}`;
      block.appendChild(line);
    }
    if (e.hull_current != null && e.hull_max != null) {
      const pct = e.hull_max > 0 ? Math.round((e.hull_current / e.hull_max) * 100) : 0;
      const line = doc.createElement('div');
      line.className = 'dbg-row';
      line.textContent = `    hull: ${Math.round(e.hull_current)}/${Math.round(e.hull_max)} (${pct}%)`;
      block.appendChild(line);
    }
    if (e.comms_range != null) {
      const line = doc.createElement('div');
      line.className = 'dbg-row';
      line.textContent = e.comms_in_range
        ? '    comms: hailable (in range)'
        : `    comms: hailable (range ${Math.round(e.comms_range)}u)`;
      block.appendChild(line);
    }
    if (e.ai_target != null) {
      const line = doc.createElement('div');
      line.className = 'dbg-row';
      line.textContent = `    ai: target=${e.ai_target}`;
      block.appendChild(line);
    }

    root.appendChild(block);
  });

  return root;
}

// ── render*(container, json) wrappers the settings cog wires in ──────────────

function makeRenderer(build, emptyKey) {
  return function render(container, json, opts = {}) {
    if (!container) return;
    const doc = container.ownerDocument || opts.doc || document;
    const payload = parseDebugPayload(json);
    container.textContent = '';
    if (!payload) {
      const empty = doc.createElement('div');
      empty.className = 'dbg-empty';
      empty.textContent = t(emptyKey);
      container.appendChild(empty);
      return;
    }
    container.appendChild(build(payload, { doc }));
  };
}

export const renderModifierDebug = makeRenderer(buildModifierDebug, 'settings.debug.output_hint');
export const renderDamageDebug = makeRenderer(buildDamageDebug, 'settings.debug.output_hint');
export const renderEntityBehaviorDebug = makeRenderer(
  buildEntityBehaviorDebug,
  'settings.debug.output_hint',
);
export const renderEntityInspectorDebug = makeRenderer(
  buildEntityInspectorDebug,
  'settings.debug.output_hint',
);

// Expose for the classic-script bootstrap in server.html.
if (typeof window !== 'undefined') {
  window.renderModifierDebug = renderModifierDebug;
  window.renderDamageDebug = renderDamageDebug;
  window.renderEntityBehaviorDebug = renderEntityBehaviorDebug;
  window.renderEntityInspectorDebug = renderEntityInspectorDebug;
}
