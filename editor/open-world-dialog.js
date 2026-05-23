/**
 * open-world-dialog.js
 *
 * Wires up the "Open World…" button (`#openWorldBtn`, owned by the editor
 * template). On click:
 *   1. listDirectory('assets/worlds') to enumerate available .toml files.
 *   2. Filter out already-open layers so you can't load the same world twice.
 *   3. Show a picker modal listing the candidates.
 *   4. On selection: readFile(path) → tomlParse → layerManager.addInMemoryLayer
 *      (non-session-only so SaveFlow can write it back to disk).
 *   5. Call onOpened() so the host can renderAll().
 */

import { isMapLayer } from './layers.js';

/**
 * Mount the "Open World…" button.
 *
 * @param {object} deps
 * @param {object} deps.layerManager
 * @param {(path:string) => Promise<{name:string, kind:string}[]>} deps.listDirectory
 * @param {(path:string) => Promise<string>} deps.readFile
 * @param {(text:string) => object} [deps.tomlParse]  Falls back to window.tomlParse.
 * @param {() => void} deps.onOpened
 * @param {string} [deps.buttonId='openWorldBtn']
 * @returns {HTMLButtonElement|null} the mounted button, or null when missing from DOM.
 */
export function mountOpenWorldButton(deps) {
  const buttonId = deps.buttonId || 'openWorldBtn';
  const btn = document.getElementById(buttonId);
  if (!btn) return null;

  btn.addEventListener('click', () => handleClick(deps));
  return btn;
}

async function handleClick(deps) {
  const { layerManager, listDirectory, readFile, onOpened } = deps;
  const tomlParse = deps.tomlParse ||
    (typeof window !== 'undefined' ? window.tomlParse : null);
  const alertFn = typeof window !== 'undefined' ? window.alert : null;

  // 1. List candidate world files.
  let entries;
  try {
    entries = await listDirectory('assets/worlds');
  } catch (err) {
    if (typeof alertFn === 'function') {
      alertFn('Could not list worlds — pick a project root first.');
    }
    return;
  }

  // 2. Exclude already-open layers.
  const existingPaths = new Set(
    layerManager.getLayers().map((l) => l.filename).filter(Boolean),
  );

  const candidates = (entries || [])
    .filter((e) => e && e.kind === 'file' && typeof e.name === 'string' && e.name.endsWith('.toml'))
    .map((e) => `assets/worlds/${e.name}`)
    .filter((p) => !existingPaths.has(p))
    .sort();

  if (!candidates.length) {
    if (typeof alertFn === 'function') {
      alertFn(
        existingPaths.size > 0
          ? 'All available worlds are already open.'
          : 'No world files found in assets/worlds/.',
      );
    }
    return;
  }

  // 3. Show picker modal.
  const chosen = await showPickerModal(candidates);
  if (!chosen) return;

  // 4. Read + parse the chosen file.
  let text;
  try {
    text = await readFile(chosen);
  } catch (err) {
    if (typeof alertFn === 'function') {
      alertFn(`Failed to read ${chosen}: ${err?.message || err}`);
    }
    return;
  }

  let parsed;
  try {
    parsed = tomlParse(text);
  } catch (err) {
    if (typeof alertFn === 'function') {
      alertFn(`TOML parse error in ${chosen}: ${err?.message || err}`);
    }
    return;
  }

  // 5. Add as a regular (non-session-only) layer so SaveFlow can write it.
  layerManager.addInMemoryLayer(chosen, parsed);

  if (typeof onOpened === 'function') onOpened();
}

/**
 * Build and show a modal that lists `paths` and resolves with the chosen
 * path, or null on cancel.
 *
 * @param {string[]} paths
 * @returns {Promise<string|null>}
 */
function showPickerModal(paths) {
  return new Promise((resolve) => {
    // Overlay
    const overlay = document.createElement('div');
    overlay.className = 'modal';
    overlay.style.display = 'flex';

    // Box
    const box = document.createElement('div');
    box.className = 'modal-content';

    const heading = document.createElement('h2');
    heading.textContent = 'Open World';
    box.appendChild(heading);

    // File list
    const list = document.createElement('div');
    list.className = 'open-world-list';

    for (const path of paths) {
      const label = path.replace('assets/worlds/', '');
      const itemBtn = document.createElement('button');
      itemBtn.className = 'open-world-item';
      itemBtn.textContent = label;
      itemBtn.addEventListener('click', () => {
        cleanup();
        resolve(path);
      });
      list.appendChild(itemBtn);
    }
    box.appendChild(list);

    // Cancel
    const actions = document.createElement('div');
    actions.className = 'form-actions';
    const cancelBtn = document.createElement('button');
    cancelBtn.type = 'button';
    cancelBtn.textContent = 'Cancel';
    cancelBtn.addEventListener('click', () => {
      cleanup();
      resolve(null);
    });
    actions.appendChild(cancelBtn);
    box.appendChild(actions);

    overlay.appendChild(box);

    // Clicking the overlay backdrop also cancels.
    overlay.addEventListener('click', (e) => {
      if (e.target === overlay) {
        cleanup();
        resolve(null);
      }
    });

    function cleanup() {
      if (overlay.parentNode) overlay.parentNode.removeChild(overlay);
    }

    document.body.appendChild(overlay);
  });
}
