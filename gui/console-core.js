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
 *  - `window.__setConsoleOverlay(id|null)` / `window.__setConsoleTabBadge(id, n)`
 *    plus the outbound `console_tabs` / `console_hull` posts — the Station Bar
 *    overlay-tab seam (issue #1373). See the block above `sendAction`.
 *
 * @param {{ name: string, render: function(state: object): void,
 *           actionFamilies?: string[], getActionContext?: function(): string,
 *           supportsSemanticAction?: function(string, string, object): boolean }} opts
 *   name   — lowercase station id (e.g. 'repair', 'helm'). Pre-issue #618
 *            these were PascalCase Console enum variant names.
 *   render — Called with the parsed (and shape-normalised) state object on
 *            every inbound push.
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
import { applyToDom, t } from './strings.js';
// Registers <ph-tutorial-overlay> (issue #916) so every console gets the
// contextual tutorial overlay without per-file HTML; Node-safe (guarded
// definition), so plain-Node test imports of this module stay fine.
import './components/ph-tutorial-overlay.js';
// The console control family (gui/components/ph-console-styles.js). Adopted
// into the document below as well as into every component's shadow root.
import { phAdoptConsoleStyles } from './components/ph-console-styles.js';
// Shape normalisation (issue #1233, T4.C1.5): every inbound payload is
// wrapped to the keyed shape here, at the one seam every console's state push
// passes through, before `render` ever sees it. See normalizeConsolePayload's
// own doc comment for the full contract.
import { normalizeConsolePayload } from './console-payload.js';
// The shell's Station Bar selects this console's overlay panels as tabs
// (issue #1373). `setConsoleOverlay` is the SET half of the overlay module —
// the bar pushes a known selection in, it does not ask the console to flip.
import { openConsoleOverlayId, setConsoleOverlay } from './console-overlays.js';
import { createSemanticActionRegistry } from './semantic-action-registry.js';
import {
  CAPTAIN_ACTION_CONTEXT,
  registerCaptainActions,
} from './stations/captain-actions.js';
import {
  HELM_ACTION_CONTEXT,
  registerHelmActions,
} from './stations/helm-actions.js';
import {
  TACTICAL_ACTION_CONTEXT,
  registerTacticalActions,
} from './stations/tactical-actions.js';
import {
  COMMS_ACTION_CONTEXT,
  registerCommsActions,
} from './stations/comms-actions.js';
import {
  SENSOR_SCIENCE_ACTION_CONTEXTS,
  registerSensorScienceActions,
} from './stations/sensors-actions.js';
import {
  NAVIGATION_ACTION_CONTEXT,
  registerNavigationActions,
} from './stations/navigation-actions.js';
import {
  ENGINEERING_ACTION_REGISTRATION_CONTEXTS,
  registerEngineeringActions,
} from './stations/engineering-actions.js';
// Console input-to-feedback latency (issue #1169, PRD #1144). `sendAction` is
// the ONE place in a console document where a control's handler turns into an
// outbound action, so it is the only honest place to stamp "the input event
// happened". See `__input_ms` in `sendAction` below.
import { nowMs } from './console-latency.js';
import {
  ACTION_FEEDBACK_PREFERENCE_DEFAULTS,
  ActionFeedbackLifecycle,
  emitActionFeedbackTransition,
  normalizeActionFeedbackPreferences,
} from './action-feedback.js';

export function initConsole({
  name,
  render,
  actionFamilies = [],
  getActionContext = null,
  supportsSemanticAction = null,
  navigationActions,
}) {
  // Resolve the global object: `window` in browsers, `globalThis` in Node/tests.
  // Evaluated at call-time so tests can set global.window before calling initConsole.
  var _root = (typeof window !== 'undefined') ? window : globalThis;
  var _actionContext = String(name || '').toLowerCase();
  var _actionContexts = new Set([
    _actionContext,
    ...(Array.isArray(actionFamilies) ? actionFamilies : [])
      .map((value) => String(value || '').toLowerCase())
      .filter(Boolean),
  ]);
  var _latestState = null;
  var _semanticActions = null;
  var _semanticFeedbackEl = null;
  var _semanticFeedbackByAction = new Map();
  var _gmTakeoverEl = null;

  function _ensureGmTakeoverElement() {
    if (_gmTakeoverEl && _gmTakeoverEl.isConnected) return;
    if (typeof document === 'undefined') return;
    _gmTakeoverEl = document.createElement('aside');
    _gmTakeoverEl.id = 'gm-takeover-banner';
    _gmTakeoverEl.className = 'gm-takeover-banner';
    _gmTakeoverEl.hidden = true;
    _gmTakeoverEl.setAttribute('role', 'status');
    _gmTakeoverEl.setAttribute('aria-live', 'polite');
    _gmTakeoverEl.setAttribute('aria-atomic', 'true');
    (document.body || document.documentElement).appendChild(_gmTakeoverEl);
  }

  function _updateGmTakeover(state) {
    const takeover = state && state.gm_takeover;
    if (!takeover || !Array.isArray(takeover.operators) || takeover.operators.length === 0) {
      if (_gmTakeoverEl) {
        _gmTakeoverEl.hidden = true;
        _gmTakeoverEl.textContent = '';
        delete _gmTakeoverEl.dataset.station;
        delete _gmTakeoverEl.dataset.latestOperator;
      }
      return;
    }
    _ensureGmTakeoverElement();
    if (!_gmTakeoverEl) return;
    const activity = takeover.latest_activity;
    const params = { operators: takeover.operators.join(', ') };
    if (activity && typeof activity === 'object') {
      params.operator = activity.operator_id || '';
      params.target = activity.target || '';
      params.action = activity.action || '';
      _gmTakeoverEl.textContent = t('console.gm_takeover.active_with_activity', params);
      _gmTakeoverEl.dataset.latestOperator = params.operator;
    } else {
      _gmTakeoverEl.textContent = t('console.gm_takeover.active', params);
      delete _gmTakeoverEl.dataset.latestOperator;
    }
    _gmTakeoverEl.dataset.station = takeover.station || name;
    _gmTakeoverEl.hidden = false;
  }

  function _ensureSemanticFeedbackElement() {
    if (_semanticFeedbackEl || typeof document === 'undefined') return;
    _semanticFeedbackEl = document.createElement('div');
    _semanticFeedbackEl.className = 'semantic-action-feedback';
    _semanticFeedbackEl.setAttribute('role', 'status');
    _semanticFeedbackEl.setAttribute('aria-live', 'polite');
    _semanticFeedbackEl.setAttribute('aria-atomic', 'true');
    (document.body || document.documentElement).appendChild(_semanticFeedbackEl);
  }

  function _renderActionFeedback() {
    _ensureSemanticFeedbackElement();
    if (!_semanticFeedbackEl) return;
    const rows = [];
    for (const value of _semanticFeedbackByAction.values()) {
      const action = _semanticActions && _semanticActions.action(value.actionId);
      const label = action ? t(action.labelId) : '';
      const row = document.createElement('div');
      row.className = 'semantic-action-feedback__item';
      row.dataset.actionId = value.actionId;
      row.dataset.state = value.state || '';
      row.textContent = t('action_feedback.summary', {
        action: label,
        status: t(value.statusId),
      });
      rows.push(row);
    }
    _semanticFeedbackEl.replaceChildren(...rows);
    if (rows.length === 1) {
      _semanticFeedbackEl.dataset.state = rows[0].dataset.state;
    } else if (rows.length > 1) {
      _semanticFeedbackEl.dataset.state = 'Mixed';
    } else {
      _semanticFeedbackEl.removeAttribute('data-state');
    }
    _semanticFeedbackEl.dataset.pendingCount = String(
      [..._semanticFeedbackByAction.values()]
        .filter((value) => value.state === 'Pending').length,
    );
  }

  function _presentActionFeedback(value) {
    emitActionFeedbackTransition(_root, value, _feedbackPreferences);
    if (typeof document === 'undefined' || !value || value.isCurrent === false) return;
    if (value.cancelled || !value.statusId) {
      _semanticFeedbackByAction.delete(value.actionId);
    } else {
      _semanticFeedbackByAction.set(value.actionId, value);
    }
    _renderActionFeedback();
  }

  // One registry per console document. The parent page owns the mutable
  // in-memory binding choices and explicitly copies them into each iframe;
  // module instances in separate realms are never treated as shared state.
  var _feedbackPreferences = ACTION_FEEDBACK_PREFERENCE_DEFAULTS;
  var _actionFeedback = new ActionFeedbackLifecycle({
    now: nowMs,
    onTransition: _presentActionFeedback,
  });
  _semanticActions = createSemanticActionRegistry({ actionFeedback: _actionFeedback });
  if (_actionContexts.has(CAPTAIN_ACTION_CONTEXT)) {
    registerCaptainActions(_semanticActions, {
      getState: function() { return _latestState; },
      getAvailableCameraViews: typeof render.availableCameraViews === 'function'
        ? function() { return render.availableCameraViews(_latestState); }
        : null,
      // This is the existing console action transport. The Captain adapter
      // emits `set_red_alert`; action-map.js remains the sole wire builder.
      sendAction: sendAction,
    });
  }
  if (_actionContexts.has(HELM_ACTION_CONTEXT)) {
    registerHelmActions(_semanticActions, {
      getState: function() { return _latestState; },
      sendAction: sendAction,
      hasThrustControl: function() {
        return typeof document !== 'undefined' && !!document.querySelector('ph-helm-joystick');
      },
      hasSteeringControl: function() {
        return typeof document !== 'undefined' && !!document.querySelector('ph-helm-joystick');
      },
      hasLateralControl: function() {
        return typeof document !== 'undefined'
          && !!document.querySelector('ph-lateral-thrust-joystick');
      },
      hasImpulseControl: function() {
        return typeof document !== 'undefined' && !!document.querySelector('ph-impulse-btn');
      },
      hasBoostControl: function() {
        return typeof document !== 'undefined' && !!document.querySelector('ph-boost-btn');
      },
      hasViewscreenControl: function() {
        return typeof document !== 'undefined' && !!document.querySelector('ph-helm-radar');
      },
      hasDockControl: function() {
        return typeof document !== 'undefined' && !!document.getElementById('dock-btn');
      },
    });
  }
  if (_actionContexts.has(TACTICAL_ACTION_CONTEXT)) {
    registerTacticalActions(_semanticActions, {
      getState: function() { return _latestState; },
      sendAction: sendAction,
    });
  }
  if (_actionContexts.has(COMMS_ACTION_CONTEXT)) {
    registerCommsActions(_semanticActions, {
      getState: function() { return _latestState; },
      // Comms message selection is presentation-only, but it is still a real
      // operation: the shared renderer owns the selected id and repaints both
      // the list and current-message panel synchronously. The semantic local
      // lifecycle settles only after this callback reports that work applied.
      selectMessage: typeof render.selectMessage === 'function'
        ? function(messageId) { return render.selectMessage(_latestState, messageId); }
        : null,
      getCurrentMessage: typeof render.currentMessage === 'function'
        ? function() { return render.currentMessage(_latestState); }
        : null,
      sendAction: sendAction,
    });
  }
  if (SENSOR_SCIENCE_ACTION_CONTEXTS.some((context) => _actionContexts.has(context))) {
    registerSensorScienceActions(_semanticActions, {
      getState: function() { return _latestState; },
      sendAction: sendAction,
    });
  }
  // Composite Stations layer family adapters onto their primary Station
  // actions. In particular the Courier Captain retains all Captain actions
  // while gaining the Power/Repair controls mounted in the same document.
  if (ENGINEERING_ACTION_REGISTRATION_CONTEXTS.some((context) => _actionContexts.has(context))) {
    registerEngineeringActions(_semanticActions, {
      getState: function() { return _latestState; },
      sendAction: sendAction,
    });
  }
  if (_actionContexts.has(NAVIGATION_ACTION_CONTEXT)) {
    registerNavigationActions(_semanticActions, {
      getState: function() { return _latestState; },
      getSurface: function() {
        if (typeof document === 'undefined' || !document.querySelectorAll) return null;
        var maps = Array.from(document.querySelectorAll('ph-navigation-map'));
        if (maps.length === 0) return null;
        var active = document.activeElement;
        var focused = maps.find(function(map) {
          return map === active || (typeof map.contains === 'function' && map.contains(active));
        });
        if (focused) return focused;
        var open = maps.find(function(map) {
          return typeof map.closest === 'function' && map.closest('.open');
        });
        if (open) return open;
        var visible = maps.find(function(map) {
          return typeof map.getClientRects !== 'function' || map.getClientRects().length > 0;
        });
        // A direct Navigation document owns its one chart even in non-layout
        // harnesses.  A composite Captain/Comms document must not fall back to
        // a hidden overlay map: the parent gamepad runtime addresses the host
        // station context and can otherwise fire an invisible Navigation
        // control. Cruiser Comms still has an always-visible inline chart, so
        // that legitimate surface is selected by the `visible` arm above.
        return visible || (_actionContexts.size === 1 ? maps[0] : null);
      },
      supportsChart: navigationActions && navigationActions.supportsChart != null
        ? navigationActions.supportsChart === true
        : typeof document === 'undefined' || !!document.getElementById('btn-on-screen'),
      supportsCivilianOrders: navigationActions && navigationActions.supportsCivilianOrders != null
        ? navigationActions.supportsCivilianOrders === true
        : typeof document === 'undefined' || !!document.querySelector('ph-civilian-traffic'),
      sendAction: sendAction,
    });
  }

  // The control family reaches this DOCUMENT, not only its components.
  //
  // Shadow DOM blocks class rules, so the buttons live in a constructable
  // stylesheet each ph-* component adopts. A few consoles also write
  // `class="btn"` in their own light DOM, and console.css used to answer that
  // with a second, differently scaled copy of the same design. Adopting the
  // one sheet into the document means both sides of every shadow boundary
  // draw the same control. Idempotent, and a no-op where there is no document.
  if (typeof document !== 'undefined') phAdoptConsoleStyles(document);

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
      // Belt-and-suspenders: PhElement's connectedCallback (run by the
      // appendChild below) already wires this via its live-reading
      // `sendAction` accessor, since `window.sendAction` is published long
      // before this lazy mount ever happens. Assigning it directly here too
      // costs nothing and keeps this call site self-contained.
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
    s = normalizeConsolePayload(s);
    _latestState = s;
    render(s);
    _updateGmTakeover(s);
    _updateTutorialOverlay(s);
    _publishConsoleHull(s);
    _syncConsoleOverlay();
  };

  // Parent -> originating iframe final feedback.  The parent routes by the
  // exact correlation it saw leave this iframe; late and duplicate replies
  // are ignored by the bounded lifecycle above.
  _root.__updateActionFeedback = function(value) {
    if (!value || typeof value !== 'object') return false;
    return _actionFeedback.settle(value.correlation, value.state);
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

  // ── Overlay tabs: this console declares them, the shell selects them ───
  // (issue #1373, PRD #1371.)
  //
  // A console's overlay panels are surfaces it has no room for inline. Before
  // this they were reached by a button inside the console's own header; the
  // Station Bar is the one header now, so the console has to SAY what it has
  // and the bar has to be able to open one.
  //
  // Three seams, all additions beside the existing pair:
  //   out  `{type:'console_tabs', console, open, tabs:[{id,code,name,badge}]}`
  //        — posted once the document is parsed, and again whenever a badge or
  //        the open panel changes. `code` and `name` are strings.csv IDS, not
  //        display text: the shell resolves them the same way it resolves a
  //        Station's own name off the wire, so no English crosses this seam.
  //   in   `__setConsoleOverlay(id|null)` — set, not toggle.
  //   in   `__setConsoleTabBadge(id, n)` — called by the console's own render.
  //
  // `open` is what keeps the bar honest. The DOCUMENT is the truth about which
  // panel is covering the console, and the shell is a remote control for it —
  // so a panel closed from inside (the panel's own Back button, a toggle the
  // console still draws) reports itself rather than leaving a tab lit over
  // nothing. The shell may still light a tab optimistically on the tap; this
  // is what it settles to.
  //
  // Posted ONLY to an iframe parent. That is the one context with a bar to
  // render tabs: server.html, the wry host and separate-tab mode would each
  // have to learn to ignore a message that can never mean anything to them.
  var _consoleTabs = [];
  var _lastHullSignature = null;
  var _lastPostedOpen = null;

  function _postToShell(message) {
    var _win = (typeof window !== 'undefined') ? window : null;
    if (!_win || _win === _win.parent) return false;
    _win.parent.postMessage(message, '*');
    return true;
  }

  function _currentOverlay() {
    return (typeof document === 'undefined') ? null : openConsoleOverlayId(document);
  }

  function _postConsoleTabs() {
    _lastPostedOpen = _currentOverlay();
    _postToShell({
      type: 'console_tabs',
      console: name,
      open: _lastPostedOpen,
      tabs: _consoleTabs.map(function(tab) {
        return { id: tab.id, code: tab.code, name: tab.name, badge: tab.badge };
      }),
    });
  }

  // Re-declare if the open panel moved without the shell asking — which is
  // exactly what the panel's own Back button does. Cheap: a selector read and
  // a string compare, on a push that already re-rendered the whole console.
  function _syncConsoleOverlay() {
    if (_currentOverlay() !== _lastPostedOpen) _postConsoleTabs();
  }

  // The declaration itself: every `.overlay-panel` that authored a tab code.
  // A panel with no `data-tab-code` is not a tab — that is how a surface opts
  // out (and how a console keeps a panel the bar should not offer).
  function _publishConsoleTabs() {
    if (typeof document === 'undefined' || !document.querySelectorAll) return;
    _consoleTabs = Array.prototype.slice
      .call(document.querySelectorAll('.overlay-panel[data-tab-code]'))
      .filter(function(panel) { return !!panel.id; })
      .map(function(panel) {
        return {
          id: panel.id,
          code: panel.dataset.tabCode || '',
          name: panel.dataset.tabName || panel.dataset.tabCode || '',
          badge: 0,
        };
      });
    _postConsoleTabs();
  }

  // The own-Station system rows, for the damage popup the bar's selected tab
  // opens (issue #1374). Sent only when they actually change: `own_hull` is
  // merged into every payload and a payload arrives ten times a second, so an
  // unconditional post would be ten identical messages per second forever.
  function _publishConsoleHull(s) {
    var entries = (s && s.own_hull && Array.isArray(s.own_hull.entries))
      ? s.own_hull.entries : [];
    var signature = JSON.stringify(entries);
    if (signature === _lastHullSignature) return;
    _lastHullSignature = signature;
    _postToShell({ type: 'console_hull', console: name, entries: entries });
  }

  // Set, not toggle (see gui/console-overlays.js). Re-renders afterwards
  // because opening a panel is a state change the console's own render owes an
  // answer to — Intel's badge clears the moment the seat looks at it, and the
  // next state push is up to a tenth of a second away.
  _root.__setConsoleOverlay = function(overlayId) {
    if (typeof document === 'undefined') return null;
    var opened = setConsoleOverlay(overlayId || null, document);
    if (_latestState) render(_latestState);
    _syncConsoleOverlay();
    return opened;
  };

  // The console's own render reports what its tab should be carrying. Only a
  // CHANGE re-posts, so a render that recomputes the same number costs nothing.
  _root.__setConsoleTabBadge = function(overlayId, count) {
    var id = String(overlayId || '');
    var value = Number(count);
    var badge = Number.isFinite(value) ? Math.max(0, Math.trunc(value)) : 0;
    var tab = _consoleTabs.find(function(entry) { return entry.id === id; });
    if (!tab || tab.badge === badge) return false;
    tab.badge = badge;
    _postConsoleTabs();
    return true;
  };

  // ── Outbound: sendAction (ADR-0001 §1 + §3) ────────────────────────────
  // Builds the standard action envelope { action, console, ...payload },
  // stringifies it, then dispatches via the first available transport.
  function sendAction(action, payload) {
    var env = Object.assign({ action: action, console: name }, payload || {});
    // The input-event stamp for console latency (issue #1169). This function is
    // called synchronously from the control's own handler, so `nowMs()` here IS
    // the input event; by the time the shell sees the postMessage, the hop it
    // measures has already happened.
    //
    // Stamped unconditionally, not behind the debug flag, because a console
    // document has no way to learn the flag: it receives state payloads, not
    // `DebugState`. The cost is one clock read per player tap — a few hundred
    // nanoseconds at human input rates — which is not observable overhead in the
    // sense PRD #1144 means (the SIMULATION takes no reading when the flag is
    // off, and the shell throws this value away).
    //
    // Underscore-prefixed and stripped at the shell: every `gui/action-map.js`
    // handler builds its outbound `ClientMessage` from NAMED fields, so this key
    // reaches the shell and stops there. It never crosses the wire.
    if (!Number.isFinite(env.__input_ms)) env.__input_ms = nowMs();
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
  // calling `this.sendAction(...)`. Every one of them extends `PhElement`
  // (issue #1236), whose `sendAction` accessor reads `window.sendAction`
  // live on every call rather than snapshotting it once — so publishing it
  // here is the whole job. (Earlier, elements captured `window.sendAction`
  // once in `connectedCallback`, which ran too early to see it — the console
  // HTML imports component modules, upgrading any already-parsed elements,
  // *before* this line runs — so a second pass re-assigning `.sendAction`
  // onto every hyphenated element already in the DOM was needed to repair
  // that stale capture. PhElement's live-reads accessor (see its own doc
  // comment) made that repair pass unnecessary; see issue #1237.)
  _root.sendAction = sendAction;

  function _resolvedActionContext(requested) {
    const explicit = String(requested || '').toLowerCase();
    if (explicit && _actionContexts.has(explicit)) return explicit;
    const current = typeof getActionContext === 'function'
      ? String(getActionContext() || '').toLowerCase()
      : '';
    return current && _actionContexts.has(current) ? current : _actionContext;
  }

  // Parent-visible composite context. The parent gamepad owner reads the
  // currently open family here, but still routes the activation back to this
  // same Station iframe through the capability predicate below.
  _root.__semanticActionContext = function() {
    return _resolvedActionContext();
  };
  _root.__supportsSemanticActionContext = function(context) {
    return _actionContexts.has(String(context || '').toLowerCase());
  };
  _root.__supportsSemanticAction = function(actionId, context) {
    const entry = _semanticActions.action(String(actionId || ''));
    if (!entry) return false;
    const resolvedContext = _resolvedActionContext(context);
    if (!entry.contexts.includes(resolvedContext)) return false;
    return typeof supportsSemanticAction !== 'function'
      || supportsSemanticAction(entry.id, resolvedContext, entry) === true;
  };

  // Semantic activation is a live document seam just like sendAction. Both a
  // visible component and the keyboard matcher call this same identity.
  _root.activateSemanticAction = function(actionId, options) {
    return _semanticActions.activate(actionId, Object.assign({}, options || {}, {
      context: _resolvedActionContext(options && options.context),
    }));
  };

  // Explicit parent → iframe binding update. This intentionally carries only
  // local presentation data; no profile or binding becomes a ClientMessage.
  _root.__updateSemanticActionBindings = function(profile) {
    return _semanticActions.updateBindings(profile);
  };

  // Explicit parent → iframe feedback-preference update. It follows the same
  // private, in-memory seam as bindings and never becomes a ClientMessage.
  // Surface capability filtering happens in the parent adapter, so a native
  // pane can retain `vibration: true` in its portable profile while suppressing
  // an output Ultralight cannot provide.
  _root.__updateActionFeedbackPreferences = function(preferences) {
    _feedbackPreferences = normalizeActionFeedbackPreferences(preferences);
    return _feedbackPreferences;
  };

  var _semanticKeyHandler = null;
  var _semanticBlurHandler = null;
  function _releaseAllKeyboardHolds() {
    for (const context of _actionContexts) {
      _semanticActions.releaseKeyboardHolds(context);
    }
  }
  if (typeof document !== 'undefined' && document.addEventListener) {
    _semanticKeyHandler = function(event) {
      _semanticActions.dispatchKeyboardEvent(event, _resolvedActionContext());
    };
    document.addEventListener('keydown', _semanticKeyHandler);
    document.addEventListener('keyup', _semanticKeyHandler);
  }
  if (_root && typeof _root.addEventListener === 'function') {
    _semanticBlurHandler = function() {
      _releaseAllKeyboardHolds();
    };
    _root.addEventListener('blur', _semanticBlurHandler);
  }

  // ── Static text (localisation) ─────────────────────────────────────────
  // Substitute every data-i18n / data-i18n-attr node in the page. Console
  // markup carries string ids, not English — this is the pass that turns
  // them into display text. Runs once at init; dynamic text goes through
  // t() inside the console's own render function instead.
  if (typeof document !== 'undefined') {
    if (document.readyState === 'loading') {
      document.addEventListener('DOMContentLoaded', function() {
        applyToDom(document);
        _publishConsoleTabs();
      });
    } else {
      applyToDom(document);
      // Same pass, and deliberately after it: the overlay declaration is read
      // off the parsed document, so it belongs wherever "the DOM is ready"
      // is already decided rather than in a second guess at the same question.
      _publishConsoleTabs();
    }
  }

  return {
    sendAction: sendAction,
    semanticActions: _semanticActions,
    disposeSemanticActions: function() {
      if (_semanticKeyHandler && typeof document !== 'undefined'
          && document.removeEventListener) {
        document.removeEventListener('keydown', _semanticKeyHandler);
        document.removeEventListener('keyup', _semanticKeyHandler);
        _semanticKeyHandler = null;
      }
      if (_semanticBlurHandler && _root && typeof _root.removeEventListener === 'function') {
        _root.removeEventListener('blur', _semanticBlurHandler);
        _semanticBlurHandler = null;
      }
      _releaseAllKeyboardHolds();
      if (_semanticFeedbackEl) {
        _semanticFeedbackEl.remove();
        _semanticFeedbackEl = null;
      }
      _semanticFeedbackByAction.clear();
    },
  };
}

// Expose for non-module HTML scripts (fallback path only — prefer the import).
if (typeof window !== 'undefined') {
  window.initConsole = initConsole;
}
