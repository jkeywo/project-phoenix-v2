/**
 * new-world-dialog.js
 *
 * Mounts a "+ New World" button next to `#addLayerBtn`. On click:
 *   1. prompt() for a filename (default "new_world.toml").
 *   2. Derive `assets/worlds/<name>`.
 *   3. validateNewWorldPath against the existing paths set.
 *   4. prepareNewWorld(path) → { content, parsedContent }.
 *   5. writeFile(path, content).
 *   6. Push the layer onto layerManager (mirroring app.js layer shape).
 *   7. Invoke onCreated() so the host can renderAll().
 */

import {
  validateNewWorldPath,
  prepareNewWorld,
  getDefaultNewWorldPath,
} from './new-world.js';
import { inferLayerKind } from './layers.js';

/**
 * Mount the "+ New World" button.
 *
 * @param {object} deps
 * @param {object} deps.layerManager
 * @param {(path:string, content:string) => Promise<void>} deps.writeFile
 * @param {(text:string) => object} [deps.tomlParse]  Falls back to window.tomlParse.
 * @param {() => void} deps.onCreated
 * @param {() => string[]} [deps.getExistingPaths]
 * @param {string} [deps.buttonId='newWorldBtn']
 * @param {string} [deps.siblingId='addLayerBtn']
 * @returns {HTMLButtonElement|null} the mounted button, or null when DOM
 *   structure is missing.
 */
export function mountNewWorldButton(deps) {
  const buttonId = deps.buttonId || 'newWorldBtn';
  const siblingId = deps.siblingId || 'addLayerBtn';

  let btn = document.getElementById(buttonId);

  // If the editor.html template already provides the button, reuse it.
  // Otherwise create + insert it next to addLayerBtn for runtime back-compat
  // with older template snapshots.
  if (!btn) {
    const sibling = document.getElementById(siblingId);
    if (!sibling) return null;
    btn = document.createElement('button');
    btn.id = buttonId;
    btn.type = 'button';
    btn.textContent = '+ New World';
    sibling.parentElement?.appendChild(btn);
  }

  btn.addEventListener('click', () => handleClick(deps));
  return btn;
}

async function handleClick(deps) {
  const defaultPath = getDefaultNewWorldPath();
  const defaultName = defaultPath.replace(/^assets\/worlds\//, '');
  const promptFn = typeof window !== 'undefined' ? window.prompt : null;
  const alertFn  = typeof window !== 'undefined' ? window.alert  : null;
  if (typeof promptFn !== 'function') return;

  const raw = promptFn('World filename:', defaultName);
  if (raw == null) return;
  const trimmed = String(raw).trim();
  if (!trimmed) return;

  // Allow either a bare filename or an explicit assets/worlds/ prefix.
  const path = trimmed.startsWith('assets/worlds/')
    ? trimmed
    : `assets/worlds/${trimmed}`;

  const existing = typeof deps.getExistingPaths === 'function'
    ? (deps.getExistingPaths() || [])
    : deps.layerManager.getLayers().map((l) => l.filename);

  const validation = validateNewWorldPath(path, existing);
  if (!validation.ok) {
    if (typeof alertFn === 'function') alertFn(validation.error);
    return;
  }

  const prepared = prepareNewWorld(path);
  if (!prepared.ok) {
    if (typeof alertFn === 'function') alertFn(prepared.error);
    return;
  }

  try {
    await deps.writeFile(path, prepared.content);
  } catch (err) {
    if (typeof alertFn === 'function') alertFn(`Write failed: ${err?.message || err}`);
    return;
  }

  // Re-parse with the host's tomlParse (smol-toml) for consistency with
  // the rest of the app. Falls back to the structure prepareNewWorld
  // produced via parseWorldToml.
  const parser = deps.tomlParse || (typeof window !== 'undefined' ? window.tomlParse : null);
  let parsed = prepared.parsedContent;
  if (typeof parser === 'function') {
    try { parsed = parser(prepared.content); } catch { /* fall through */ }
  }

  const layer = {
    fileHandle: null,
    filename: path,
    toml: parsed,
    kind: inferLayerKind(parsed),
    visible: true,
    active: true,
    konvaLayer: null,
    originalText: prepared.content,
    isDirty: false,
  };
  deps.layerManager.layers.push(layer);
  deps.layerManager.activeLayer = layer;

  if (typeof deps.onCreated === 'function') deps.onCreated();
}
