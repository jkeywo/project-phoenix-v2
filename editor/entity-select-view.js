/**
 * entity-select-view.js
 *
 * Shared helper that renders the entity-name `<select>` dropdown used by
 * action cards (`action-card-view.js`), trigger headers (`trigger-view.js`),
 * and comms templates (`comms-view.js`).
 *
 * Behaviour:
 *   - Options sourced via `getEntityNameOptions(allLayers)`.
 *   - Deduplication key is `value + ':' + (layerPath || '')` — NOT bare
 *     `value`. This preserves multiple `<option>` entries when the same
 *     entity name appears in more than one open layer (collision-suffix
 *     labels stay visible).
 *   - `<option>.value` is the bare `name` (the wire format). Only
 *     `option.textContent` carries the collision-suffixed label.
 *   - Unknown-name warning: if the saved value is non-empty and not in any
 *     option, a `(value) ⚠ unknown` option is appended and a sibling
 *     `<span class="action-field-warning">⚠ unknown</span>` is rendered.
 */

import { getEntityNameOptions } from './trigger-pickers.js';

/**
 * Render an entity-name dropdown into `host`.
 *
 * @param {HTMLElement} host          Container to append into.
 * @param {string|null|undefined} value  Current saved entity name.
 * @param {Array<{path:string, worldState:object}>} allLayers
 * @param {(newValue: string) => void} onChange  Called with the new value
 *                                                whenever the `<select>` fires
 *                                                a `change` event.
 * @param {object} [opts]
 * @param {boolean} [opts.includeNone=true]  Prepend a `(none)` placeholder.
 * @returns {HTMLElement} The wrapper span (also already appended to host).
 */
export function renderEntitySelect(host, value, allLayers, onChange, opts = {}) {
  const includeNone = opts.includeNone !== false;

  const wrap = document.createElement('span');
  wrap.style.display = 'flex';
  wrap.style.gap = '6px';
  wrap.style.flex = '1';

  const select = document.createElement('select');

  if (includeNone) {
    const empty = document.createElement('option');
    empty.value = '';
    empty.textContent = '(none)';
    select.appendChild(empty);
  }

  const opts2 = getEntityNameOptions(allLayers || []);
  const seenKeys = new Set();
  const knownValues = new Set();
  for (const o of opts2) {
    const key = `${o.value}:${o.layerPath || ''}`;
    if (seenKeys.has(key)) continue;
    seenKeys.add(key);
    knownValues.add(o.value);
    const opt = document.createElement('option');
    opt.value = o.value;
    opt.textContent = o.label;
    select.appendChild(opt);
  }

  const isUnknown = value != null && value !== '' && !knownValues.has(value);
  if (isUnknown) {
    const opt = document.createElement('option');
    opt.value = value;
    opt.textContent = `${value} ⚠ unknown`;
    select.appendChild(opt);
  }

  select.value = value ?? '';

  select.addEventListener('change', (e) => {
    if (typeof onChange === 'function') onChange(e.target.value);
  });
  wrap.appendChild(select);

  if (isUnknown) {
    const warn = document.createElement('span');
    warn.className = 'action-field-warning';
    warn.textContent = '⚠ unknown';
    wrap.appendChild(warn);
  }

  host.appendChild(wrap);
  return wrap;
}
