/**
 * gui/console-core.js — Shared console runtime for HTML console panels.
 *
 * Replaces the copy-pasted transport shim and `window.__updateConsole`
 * boilerplate that previously appeared in every console HTML file. See
 * ADR-0001 for the full contract; this module implements §2 (inbound
 * state push) and §3 (outbound action transport).
 *
 * Usage in a console HTML file:
 *
 *   <script type="module">
 *     import { initConsole } from './console-core.js';
 *     const { sendAction } = initConsole({
 *       name: 'repair',          // lowercase station id (issue #618)
 *       render: function(state) { ... },  // receives the parsed state object
 *     });
 *     // Use sendAction('action_name', { ...payload }) instead of
 *     // window.__sendAction(JSON.stringify({ action, console, ...payload }))
 *   </script>
 */

/**
 * Initialise the shared console runtime for one HTML console.
 *
 * Sets up:
 *  - `window.__updateConsole(name, stateJson)` — inbound push (ADR-0001 §2).
 *    JSON-parses stateJson and calls `render` with the plain object.
 *    Logs a console.warn (tagged with the console name) on parse failure.
 *  - BroadcastChannel listener on 'phoenix-console-state', filtering by
 *    `name` — for same-origin separate-tab mode (ADR-0001 §3 target 4).
 *
 * @param {{ name: string, render: function(state: object): void }} opts
 *   name   — lowercase station id (e.g. 'repair', 'helm'). Pre-issue #618
 *            these were PascalCase Console enum variant names.
 *   render — Called with the parsed state object on every inbound push.
 *
 * @returns {{ sendAction: function(action: string, payload?: object): void }}
 *   sendAction — Outbound action dispatcher. Injects `console: name` and
 *   stringifies, then routes via the 4-way transport detection:
 *     1. iframe postMessage  (running inside client.html)
 *     2. window.ipc          (wry native host)
 *     3. window.__sendAction (browser WASM host page — server.html routes the
 *        envelope through gui/action-map.js, issue #822)
 *     4. BroadcastChannel    (separate-tab mode)
 */
// strings-boot's top-level await blocks this module (and therefore every
// console page) until the string table is loaded, so data-i18n substitution
// below and t() calls in console render functions never see an empty table.
// In Node tests strings-boot is a no-op; setup-strings.js loads the table.
import './strings-boot.js';
import { applyToDom } from './strings.js';
import { mountHelp } from './help-panel.js';
// Registers <ph-tutorial-overlay> (issue #916) so every console gets the
// contextual tutorial overlay without per-file HTML; Node-safe (guarded
// definition), so plain-Node test imports of this module stay fine.
import './components/ph-tutorial-overlay.js';

export function initConsole({ name, render }) {
  // Resolve the global object: `window` in browsers, `globalThis` in Node/tests.
  // Evaluated at call-time so tests can set global.window before calling initConsole.
  var _root = (typeof window !== 'undefined') ? window : globalThis;

  // The console can run in four contexts (ADR-0001 §3 transport targets):
  //   1. Inside a `client.html` iframe — parent owns the push contract
  //      and calls `iframeEl.contentWindow.__updateConsole` directly.
  //   2. Inside a wry native webview — host calls `__updateConsole` via
  //      `webview.evaluate_script`.
  //   3. Inside a browser WASM page — the host page calls `__updateConsole`
  //      directly.
  //   4. As its own browser tab — same-origin server.html broadcasts state
  //      on `BroadcastChannel('phoenix-console-state')`.
  //
  // Only target 4 needs the BroadcastChannel inbound listener. In contexts
  // 1-3 a direct caller already owns __updateConsole, and turning on the
  // BC listener creates a SECOND state source that races with the direct
  // push (e.g. helm iframe receiving server.html's minimal HelmConsoleState
  // alternating with client.html's full state-with-blips — see #482).
  //
  // Match the same priority order as outbound (sendAction below): the
  // BC listener is only attached when there is no parent / no wry host /
  // no browser-WASM bindings.
  var _hasParent     = (typeof window !== 'undefined') && window !== window.parent;
  var _hasWryHost    = (typeof window !== 'undefined') && !!window.ipc;
  var _hasWasmHost   = (typeof window !== 'undefined') && !!(window.wasmBindings
    && typeof window.wasmBindings.wasm_receive_message === 'function');
  var _useBroadcastInbound = !_hasParent && !_hasWryHost && !_hasWasmHost;

  var _bc = (_useBroadcastInbound && typeof BroadcastChannel !== 'undefined')
    ? new BroadcastChannel('phoenix-console-state')
    : null;

  // ── Contextual tutorial overlay (issue #916) ───────────────────────────
  // Every console renders the tutorial block for free: the parent merges a
  // `tutorial` field into each payload (withTutorialOverlay in
  // gui/console-state.js), and this lazily mounts <ph-tutorial-overlay> the
  // first time a payload actually carries one. No per-console HTML needed —
  // authoring `[[station.tutorial]]` in the ship TOML is the whole job.
  var _tutorialEl = null;
  function _updateTutorialOverlay(s) {
    if (typeof document === 'undefined' || typeof customElements === 'undefined') return;
    if (!_tutorialEl) {
      if (!s || !s.tutorial) return; // nothing to show yet — don't mount
      _tutorialEl = document.createElement('ph-tutorial-overlay');
      // Mounted after _wireComponents ran at init, so repair the sendAction
      // reference by hand like _wireComponents would have.
      _tutorialEl.sendAction = sendAction;
      var host = document.querySelector('.frame') || document.body || document.documentElement;
      host.appendChild(_tutorialEl);
    }
    _tutorialEl.state = (s && s.tutorial) || null;
  }

  // ── Inbound: __updateConsole (ADR-0001 §2) ─────────────────────────────
  _root.__updateConsole = function(consoleName, stateJson) {
    var s;
    try { s = JSON.parse(stateJson); } catch (e) {
      console.warn('[' + name + '] bad state json', e);
      return;
    }
    render(s);
    _updateTutorialOverlay(s);
  };

  // ── BroadcastChannel receive path (ADR-0001 §3 target 4) ───────────────
  if (_bc) {
    _bc.onmessage = function(e) {
      if (e.data && e.data.type === 'console_state' && e.data.name === name) {
        if (typeof _root.__updateConsole === 'function') {
          _root.__updateConsole(e.data.name, e.data.json);
        }
      }
    };
  }

  // ── Outbound: sendAction (ADR-0001 §1 + §3) ────────────────────────────
  // Builds the standard action envelope { action, console, ...payload },
  // stringifies it, then dispatches via the first available transport.
  function sendAction(action, payload) {
    var env = Object.assign({ action: action, console: name }, payload || {});
    var json = JSON.stringify(env);
    // Re-resolve window each call so tests can swap out global.window per test.
    var _win = (typeof window !== 'undefined') ? window : null;
    if (_win && _win !== _win.parent) {
      _win.parent.postMessage({ type: 'console_action', payload: json }, '*');
    } else if (_win && _win.ipc) {
      _win.ipc.postMessage(json);
    } else if (_win && typeof _win.__sendAction === 'function') {
      // Browser WASM host page (server.html): __sendAction dispatches the
      // envelope through gui/action-map.js → ClientMessage (issue #822).
      _win.__sendAction(json);
    } else if (_bc) {
      _bc.postMessage({ type: 'console_action', payload: json });
    }
  }

  // ── Expose sendAction to web-component controls ────────────────────────
  // The `gui/components/ph-*.js` custom elements dispatch user actions by
  // calling `this.sendAction(...)`, falling back to `window.sendAction` in
  // their `connectedCallback`. Because the console HTML imports the component
  // modules *before* calling initConsole, each element's connectedCallback has
  // already run (and captured an undefined `window.sendAction`) by the time we
  // get here. So we must both (a) publish `window.sendAction` for any element
  // that reads it lazily or upgrades later, and (b) assign `.sendAction`
  // directly onto every custom element already in the DOM so their captured
  // reference is repaired. Without this every control in the new per-ship
  // consoles is inert (see the missing wiring vs. the old *-console.html files
  // which captured `initConsole(...).sendAction` and assigned it by hand).
  _root.sendAction = sendAction;
  function _wireComponents() {
    if (typeof document === 'undefined') return;
    var all = document.querySelectorAll('*');
    for (var i = 0; i < all.length; i++) {
      var el = all[i];
      // Custom elements always contain a hyphen in their tag name.
      if (el.tagName && el.tagName.indexOf('-') !== -1) {
        el.sendAction = sendAction;
      }
    }
  }
  if (typeof document !== 'undefined') {
    if (document.readyState === 'loading') {
      document.addEventListener('DOMContentLoaded', _wireComponents);
    } else {
      _wireComponents();
    }
  }

  // ── Help system (issue #462) ───────────────────────────────────────────
  // Mount the shared "?" help button + click-to-dismiss modal for this
  // console. `name` is the lowercase station id (post issue #618), which
  // doubles as the HelpPanel key. Runs only in a real DOM (guarded inside
  // mountHelp); a no-op in Node tests. Deferred until the DOM is ready so
  // the trigger host (`.frame`) exists.
  if (typeof document !== 'undefined') {
    if (document.readyState === 'loading') {
      document.addEventListener('DOMContentLoaded', function() { mountHelp(name); });
    } else {
      mountHelp(name);
    }
  }

  // ── Static text (localisation) ─────────────────────────────────────────
  // Substitute every data-i18n / data-i18n-attr node in the page. Console
  // markup carries string ids, not English — this is the pass that turns
  // them into display text. Runs once at init; dynamic text goes through
  // t() inside the console's own render function instead.
  if (typeof document !== 'undefined') {
    if (document.readyState === 'loading') {
      document.addEventListener('DOMContentLoaded', function() { applyToDom(document); });
    } else {
      applyToDom(document);
    }
  }

  return { sendAction: sendAction };
}

// Expose for non-module HTML scripts (fallback path only — prefer the import).
if (typeof window !== 'undefined') {
  window.initConsole = initConsole;
}

/**
 * Render (or hide) the per-console damage bar.
 *
 * @param {object|null} h         - own_hull from state: { current, max_hp } or null
 * @param {string} wrapId         - id of the wrapper element to show/hide
 * @param {string} fillId         - id of the bar fill element
 * @param {string} valId          - id of the text value element
 */
export function renderDamageBar(h, wrapId, fillId, valId) {
  var wrap = (typeof document !== 'undefined') && document.getElementById(wrapId);
  if (!wrap) return;
  if (!h || h.max_hp <= 0) { wrap.style.display = 'none'; return; }
  wrap.style.display = '';
  var pct  = h.current / h.max_hp;
  var fill = document.getElementById(fillId);
  var val  = document.getElementById(valId);
  if (fill) {
    fill.style.width = (Math.max(0, Math.min(1, pct)) * 100).toFixed(1) + '%';
    fill.className   = 'fill' + (pct <= 0.33 ? ' crit' : pct <= 0.60 ? ' warn' : '');
  }
  if (val) {
    val.textContent = Math.round(h.current) + '/' + Math.round(h.max_hp);
    val.className   = 'val' + (pct <= 0.33 ? ' crit' : pct <= 0.60 ? ' warn' : '');
  }
}

/**
 * Render (or hide) the station aggregate damage bar and detail popup.
 *
 * @param {object|null} agg       - aggregateStationHull result:
 *                                  { entries, totalCurrent, totalMax, pct, damagePct }
 *                                  or null / undefined when no data is available.
 * @param {string} wrapId         - id of the bar wrapper element to show/hide
 * @param {string} fillId         - id of the bar fill element
 * @param {string} valId          - id of the text value element
 * @param {string} popupId        - id of the detail popup container element
 *                                  (rendered with per-system rows when the bar is shown)
 */
export function renderStationDamageBar(agg, wrapId, fillId, valId, popupId) {
  var wrap = (typeof document !== 'undefined') && document.getElementById(wrapId);
  if (!wrap) return;
  if (!agg || agg.totalMax <= 0) { wrap.style.display = 'none'; return; }
  wrap.style.display = '';
  var pct  = agg.pct;
  var fill = document.getElementById(fillId);
  var val  = document.getElementById(valId);
  if (fill) {
    fill.style.width = (Math.max(0, Math.min(1, pct)) * 100).toFixed(1) + '%';
    fill.className   = 'fill' + (pct <= 0.33 ? ' crit' : pct <= 0.60 ? ' warn' : '');
  }
  if (val) {
    val.textContent = Math.round(agg.totalCurrent) + '/' + Math.round(agg.totalMax);
    val.className   = 'val' + (pct <= 0.33 ? ' crit' : pct <= 0.60 ? ' warn' : '');
  }
  var popup = popupId && document.getElementById(popupId);
  if (popup) {
    var rows = (agg.entries || []).map(function(e) {
      var tier = e.tier || 'Operational';
      var cls  = tier === 'Destroyed' ? ' crit'
               : tier === 'Disabled'  ? ' crit'
               : tier === 'Damaged'   ? ' warn' : '';
      return '<div class="hull-sys-row">'
           + '<span class="hull-sys-name">' + _escHtml(e.display_name || e.system_id) + '</span>'
           + '<span class="hull-sys-tier' + cls + '">' + _escHtml(tier) + '</span>'
           + '</div>';
    }).join('');
    popup.innerHTML = rows;
    popup.style.display = rows ? '' : 'none';
  }
}

function _escHtml(s) {
  return String(s).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;');
}
