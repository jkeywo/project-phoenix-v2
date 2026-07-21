import { t } from './strings.js';

/**
 * gui/console-ui.js — Shared authoring library for console iframe UI.
 *
 * Six pure DOM-manipulation primitives used across all 9 console iframes.
 * No global DOM dependency — elements are passed directly or the document
 * root is passed explicitly where needed.
 *
 * All functions are side-effect-free with respect to module state so they
 * can be unit-tested in Node via Vitest with jsdom.
 */

/**
 * Stable-ref keyed row reconciliation.
 *
 * Adds/removes/reorders child rows inside `container` to match `newIds`,
 * then calls `updateFn` for every row in the new order.
 *
 * @param {HTMLElement} container - parent element
 * @param {string[]} newIds - ordered list of row keys
 * @param {Object} cache - mutable cache object { [id]: { row, ...elements } }
 * @param {function(string): {row: HTMLElement, [key: string]: HTMLElement}} buildFn
 * @param {function(string, Object, any): void} updateFn - called with (id, cachedEls, datum)
 * @param {any[]} [data] - optional parallel data array (same order as newIds)
 */
export function reconcileRows(container, newIds, cache, buildFn, updateFn, data) {
  // 1. Remove stale rows
  for (const id of Object.keys(cache)) {
    if (!newIds.includes(id)) {
      cache[id].row.remove();
      delete cache[id];
    }
  }
  // 2. Build new rows
  for (let i = 0; i < newIds.length; i++) {
    const id = newIds[i];
    if (!cache[id]) {
      cache[id] = buildFn(id);
      container.appendChild(cache[id].row);
    }
  }
  // 3. Re-order (move nodes to match newIds order)
  for (let i = 0; i < newIds.length; i++) {
    const el = cache[newIds[i]].row;
    if (container.children[i] !== el) container.insertBefore(el, container.children[i] || null);
  }
  // 4. Update every row
  for (let i = 0; i < newIds.length; i++) {
    const id = newIds[i];
    updateFn(id, cache[id], data ? data[i] : undefined);
  }
}

/**
 * Key-guarded full-rebuild.
 *
 * Rebuilds container subtree only when newKey changes.
 *
 * @param {HTMLElement} container
 * @param {string|number} newKey
 * @param {function(HTMLElement): void} buildFn - called with container when key changes
 */
export function keyedRebuild(container, newKey, buildFn) {
  if ((container.dataset.consoleUiKey || '') !== String(newKey)) {
    container.innerHTML = '';
    container.dataset.consoleUiKey = String(newKey);
    buildFn(container);
  }
}

/**
 * Unified button state.
 *
 * @param {HTMLElement} el
 * @param {{ enabled?: boolean, active?: boolean, variant?: string, base?: string }} opts
 *   variant: extra class applied when enabled (e.g. 'danger', 'armed')
 *   base: base class string (default 'btn')
 */
export function setBtn(el, { enabled = true, active = false, variant = '', base = 'btn' } = {}) {
  el.disabled = !enabled;
  const classes = [base];
  if (variant && enabled) classes.push(variant);
  if (active) classes.push('active');
  if (!enabled) classes.push('disabled');
  const newCls = classes.join(' ');
  if (el.className !== newCls) el.className = newCls;
}

/**
 * Fill-bar percentage update with optional threshold class.
 *
 * @param {HTMLElement} fillEl
 * @param {number} fraction - 0..1
 * @param {{ thresholds?: [number, string][], baseClass?: string }} opts
 *   thresholds: array of [threshold, className] in ascending order.
 *   Default thresholds: [[0.33, 'crit'], [0.60, 'warn']]
 */
export function setBar(fillEl, fraction, { thresholds = [[0.33, 'crit'], [0.60, 'warn']], baseClass = 'fill' } = {}) {
  const pct = Math.max(0, Math.min(1, fraction));
  fillEl.style.width = (pct * 100).toFixed(1) + '%';
  let cls = baseClass;
  for (const [t, c] of thresholds) {
    if (pct <= t) { cls = baseClass + ' ' + c; break; }
  }
  if (fillEl.className !== cls) fillEl.className = cls;
}

/**
 * AI-control AUTO badge indicator.
 *
 * @param {HTMLElement|null} button - primary action button (may be null)
 * @param {HTMLElement|null} badge - AUTO badge element
 * @param {boolean} isAuto
 */
export function setAutoState(button, badge, isAuto) {
  if (button) {
    button.disabled = isAuto;
    button.classList.toggle('readonly', isAuto);
  }
  if (badge) {
    badge.hidden = !isAuto;
    badge.textContent = t('console.common.auto');
  }
}

/**
 * Text content + optional class/colour update.
 *
 * @param {HTMLElement|null} el
 * @param {string} text
 * @param {{ cls?: string, color?: string }} opts
 */
export function setText(el, text, { cls, color } = {}) {
  if (!el) return;
  if (el.textContent !== String(text)) el.textContent = String(text);
  if (cls !== undefined && el.className !== cls) el.className = cls;
  if (color !== undefined) el.style.color = color;
}
