/**
 * triggerable-worlds-panel.js
 *
 * Renders the "TRIGGERABLE WORLDS" collapsible panel into
 * `#triggerableWorldsList`. Lists candidate world TOMLs the user can
 * activate as session-only layers, so they can author and tweak triggers
 * that target entities living in those worlds.
 *
 * Candidate set:
 *   1. Every `assets/worlds/*.toml` returned by listDirectory (if a
 *      project root has been picked).
 *   2. Every path referenced by a `load_world` trigger action in any
 *      already-open layer (via `TriggerableWorlds.scanLayers`). These
 *      get a `(referenced)` badge.
 *
 * Activation:
 *   - readFile(path) → window.tomlParse → layerManager.addInMemoryLayer(
 *     path, parsedToml, { sessionOnly: true })
 *   - "Active (session)" badge shown.
 *
 * Deactivation:
 *   - Only allowed for layers whose `_sessionOnly === true`.
 *   - Removes from layerManager.layers via removeLayer.
 *
 * Defensive: if no project root picked AND no layers open, render a
 * placeholder pointing the user at "Pick Project Root".
 */

import { TriggerableWorlds } from './triggerable-worlds.js';

/**
 * Render the Triggerable Worlds panel.
 *
 * @param {object} deps
 * @param {object} deps.layerManager
 * @param {() => void} deps.onLayersChanged
 * @param {(path: string) => Promise<string>} deps.readFile
 * @param {(path?: string) => Promise<Array<{name:string, kind:string}>>} deps.listDirectory
 * @param {(text: string) => object} deps.tomlParse
 * @param {string} [deps.containerId='triggerableWorldsList']  Override for tests.
 */
export async function renderTriggerableWorldsPanel(deps) {
  const containerId = deps.containerId || 'triggerableWorldsList';
  const container = document.getElementById(containerId);
  if (!container) return;
  container.innerHTML = '';

  const layers = deps.layerManager.getLayers();
  const openPathSet = new Set(layers.map((l) => l.filename));

  // 1. Worlds reachable via load_world actions in open layers.
  const tw = new TriggerableWorlds();
  const v2Layers = layers.map((l) => ({ path: l.filename, worldState: l.toml }));
  tw.scanLayers(v2Layers);
  const referencedPaths = new Set(tw.getPaths());

  // 2. Worlds living under assets/worlds/.
  let dirEntries = null;
  if (typeof deps.listDirectory === 'function') {
    try {
      dirEntries = await deps.listDirectory('assets/worlds');
    } catch (err) {
      dirEntries = null;
    }
  }
  const diskPaths = new Set();
  if (Array.isArray(dirEntries)) {
    for (const entry of dirEntries) {
      if (entry.kind === 'file' && entry.name.endsWith('.toml')) {
        diskPaths.add(`assets/worlds/${entry.name}`);
      }
    }
  }

  const allCandidates = new Set([...diskPaths, ...referencedPaths]);

  // Placeholder: no root picked AND nothing referenced.
  if (allCandidates.size === 0 && layers.length === 0) {
    const p = document.createElement('p');
    p.className = 'placeholder';
    p.textContent = 'Pick project root to see triggerable worlds';
    container.appendChild(p);
    return;
  }

  // Render one row per candidate path, then any session-only layer that
  // somehow isn't in the candidate set (rare, but defensive).
  const allPaths = new Set(allCandidates);
  for (const layer of layers) {
    if (layer._sessionOnly) allPaths.add(layer.filename);
  }
  const sortedPaths = [...allPaths].sort();

  for (const path of sortedPaths) {
    const row = makeRow(path, {
      isOpen: openPathSet.has(path),
      isSessionOnly: layers.some((l) => l.filename === path && l._sessionOnly),
      isReferenced: referencedPaths.has(path),
      deps,
    });
    container.appendChild(row);
  }
}

function makeRow(path, { isOpen, isSessionOnly, isReferenced, deps }) {
  const row = document.createElement('div');
  row.className = 'triggerable-world-row';
  row.dataset.path = path;

  const label = document.createElement('span');
  label.className = 'triggerable-world-path';
  label.textContent = path.replace(/^assets\/worlds\//, '');
  row.appendChild(label);

  if (isSessionOnly) {
    const badge = document.createElement('span');
    badge.className = 'badge badge-session';
    badge.textContent = 'Active (session)';
    row.appendChild(badge);
  } else if (isOpen) {
    const badge = document.createElement('span');
    badge.className = 'badge badge-open';
    badge.textContent = 'Open';
    row.appendChild(badge);
  }
  if (isReferenced) {
    const badge = document.createElement('span');
    badge.className = 'badge badge-referenced';
    badge.textContent = '(referenced)';
    row.appendChild(badge);
  }

  const btn = document.createElement('button');
  btn.type = 'button';
  btn.className = 'btn-toggle-world';
  if (isSessionOnly) {
    btn.textContent = 'Deactivate';
    btn.addEventListener('click', () => deactivate(path, deps));
  } else if (isOpen) {
    // File-backed layer already open: no toggle.
    btn.textContent = 'Open';
    btn.disabled = true;
  } else {
    btn.textContent = 'Activate';
    btn.addEventListener('click', () => activate(path, deps));
  }
  row.appendChild(btn);
  return row;
}

async function activate(path, deps) {
  try {
    const text = await deps.readFile(path);
    const parsed = await deps.tomlParse(text);
    deps.layerManager.addInMemoryLayer(path, parsed, { sessionOnly: true });
    if (typeof deps.onLayersChanged === 'function') deps.onLayersChanged();
  } catch (err) {
    if (typeof window !== 'undefined' && typeof window.alert === 'function') {
      window.alert(`Failed to activate world: ${err?.message || err}`);
    }
  }
}

function deactivate(path, deps) {
  const layers = deps.layerManager.getLayers();
  const layer = layers.find((l) => l.filename === path && l._sessionOnly);
  if (!layer) return;
  deps.layerManager.removeLayer(layer);
  if (typeof deps.onLayersChanged === 'function') deps.onLayersChanged();
}
