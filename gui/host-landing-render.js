/**
 * gui/host-landing-render.js — the landing screen's renderer (issue #1360).
 *
 * The DOM half of the pair whose pure half is `gui/host-landing-view.js`:
 * `landingViewModel()` decides what the landing offers and which route is
 * open, this writes it into a document. It is the exact sibling of
 * `gui/host-scenario-render.js` (#1328) and `gui/host-lobby-render.js`
 * (#1325), and is read the same way — no state of its own, no transport, no
 * arbiter. Given a view model it writes a document, and everything it cannot
 * do itself it asks the caller for.
 *
 * It is written as a shared renderer from its first line rather than as
 * server.html glue somebody lifts out later, because the surface that will
 * need it already exists: #1361 puts this same landing on the native host's
 * viewscreen window, and a second implementation of these element ids is a
 * second landing that drifts from this one the first time either is touched.
 * That is the lesson #1325/#1328/#1329 each paid for once.
 *
 * ## `doc` is the first argument
 *
 * One renderer, two documents. Everything below reaches elements through the
 * `doc` it is handed and never through a free `document`.
 *
 * ## Every write is guarded on the element existing
 *
 * This is not defensive habit, it is the two-document contract: the native
 * lobby document carries a trimmed subset of this markup (it has no settings
 * cog, and its fullscreen control is the window manager's), so a branch whose
 * element is absent must do nothing rather than throw and abandon the rest of
 * the render half-written. The suite drives a deliberately incomplete document
 * for exactly this.
 *
 * ## `t` is injected, never imported
 *
 * The two consumers reach the string table differently: `server.html` holds a
 * classic-script `t()` closed over `window.phStrings`, and the native lobby
 * document imports `gui/strings.js` directly. Neither is this module's
 * business, so the resolver arrives as an argument — the same reason
 * `renderHostLobby` and `renderHostScenarios` take one.
 *
 * ## Side effects arrive as hooks
 *
 * A menu click has to reach whatever owns the open-entry memory, and the two
 * surfaces own it in different places. Fullscreen is already implemented once,
 * in `gui/page-chrome.js`'s `initFullscreen` — so the landing's corner control
 * does not toggle anything itself; the caller hands over a hook that reaches
 * that one implementation, the way three separate callers reach one
 * `gui/host-qr.js` toggle.
 *
 * ## The menu is rebuilt, so focus is carried across the rebuild
 *
 * Every call replaces every entry button, and the caller re-renders on every
 * click — so the node an operator was standing on is destroyed by their own
 * activation of it, and focus falls to `<body>`. On a keyboard or a gamepad
 * that is the front door losing the operator's place and restarting the tab
 * order at the top of the document. The entry id is captured before the
 * rebuild and re-focused after it, and only when the rebuild is what took
 * focus away (see below).
 *
 * ## What it does NOT draw
 *
 * The settings cog. `gui/server-settings.js` mounts its own `#server-settings-btn`
 * fixed at z-index 210, above this panel's 205, so it is already on top of the
 * landing and drawing a second one would be two cogs disagreeing about which
 * is open. Nor does the landing reserve a keep-out for it the way `#world-list`
 * does in `gui/host-scenarios.css`: the cog's corner falls inside
 * `.landing-rail`, whose content sits at the far end, and everything else
 * starts below it — the clearance is layout, and
 * `tests/smoke/server-settings-cog.spec.js` measures it.
 */

/** Lifecycle class worn by every menu entry this renderer creates. */
export const LANDING_ENTRY_CLASS = 'landing-entry';

/**
 * Everything this renderer OWNS inside `#landing-menu` and rebuilds each call.
 *
 * A lifecycle hook, separate from the styling class `landing-mi`, for the
 * reason `SCENARIO_ENTRY_SELECTOR` is `.scenario-entry` and not `.world-btn`
 * (issue #951): a styling class also worn by static markup makes a rebuild
 * delete controls it does not own. Anything added to `#landing-menu` that this
 * renderer did not create must stay out of this selector.
 */
export const LANDING_ENTRY_SELECTOR = '.landing-entry';

/** The attribute a menu entry carries its row's id in. */
export const LANDING_ENTRY_ATTR = 'data-landing-entry';

/** The class `#scenario-panel` wears while it is docked in the landing. */
export const PICKER_DOCKED_CLASS = 'landing-docked';

/**
 * Every lifecycle class this renderer can write on `#landing-panel`.
 *
 * Removed in full before the view model's own are added, so a stage that no
 * longer applies cannot be left behind by a shorter `rootClass`. Named as a
 * list rather than inline in the call for the reason the entry SELECTOR is
 * named: the stylesheet and this module have to agree on the set, and one
 * place to read it is what makes that checkable.
 */
export const LANDING_ROOT_CLASSES = ['is-idle', 'is-open', 'is-deep'];

/** Remove every entry this renderer owns, leaving anything else alone. */
export function clearLandingEntries(menu) {
  menu.querySelectorAll(LANDING_ENTRY_SELECTOR).forEach(function (el) { el.remove(); });
}

/** Write `text` into `id` if this document has it. */
function setText(doc, id, text) {
  const el = doc.getElementById(id);
  if (el) el.textContent = text;
}

/**
 * Render one landing view model into `doc`.
 *
 * @param {Document} doc the document holding the `#landing-panel` markup.
 * @param {object} vm the return of `landingViewModel()`.
 * @param {(id: string, params?: object) => string} t string-id resolver.
 * @param {{
 *   pick?: (entryId: string) => void,
 *   toggleFullscreen?: () => void,
 * }} [hooks]
 *   `pick` carries an operator's click on a menu entry back to whoever owns
 *   the open-entry memory — which is the caller, not this module (see
 *   `landingViewModel`'s note on `openEntryId`). It is called for EVERY entry
 *   including the inert ones, because whether an entry does anything is the
 *   view model's decision (`nextOpenEntry`) and not a second judgement made
 *   here. `toggleFullscreen` reaches `gui/page-chrome.js`'s one fullscreen
 *   implementation; absent, the control renders and does nothing.
 * @param {{dockPicker?: boolean, ownPanelVisibility?: boolean}} [opts]
 *   `dockPicker: false` leaves `#scenario-panel` where it is, for a surface
 *   that composes the picker some other way. `server.html` passes nothing and
 *   gets the docking; so does the native viewscreen (#1361), because the
 *   picker it carries is the same borrowed `#scenario-panel` node and
 *   `gui/host-landing.css`'s `.landing-docked` block is what unwinds it there
 *   too — a second arrangement would be a second landing.
 *
 *   `ownPanelVisibility` makes this renderer show and hide `#landing-panel`
 *   itself, from `vm.stage === 'dismissed'`. It is the exact sibling of
 *   `renderHostScenarios`'s option of the same name and exists for the same
 *   reason: on the host PAGE the landing's visibility is page lifecycle
 *   (`hideLanding()` / `showLandingAtPicker()`) and is not driven by this
 *   render at all, so `server.html` passes nothing and keeps the behaviour it
 *   had. The native viewscreen has no page lifecycle to speak of — its host
 *   is the only thing that knows a World has landed — so it asks this
 *   renderer to own the panel and pushes `dismissed` in the view model.
 */
export function renderHostLanding(doc, vm, t, hooks, opts) {
  const h = hooks || {};
  const dockPicker = !(opts && opts.dockPicker === false);
  const ownPanelVisibility = !!(opts && opts.ownPanelVisibility);

  // ── The root says only WHICH stage is open ──────────────────────────
  //
  // Not what that looks like. Wide gives three columns with the menu centred
  // until a stage opens; narrow or portrait gives one pane at a time and
  // slides the track by `--landing-depth`. Both are decided in
  // gui/host-landing.css at its breakpoints, so an orientation change relays
  // out without a single line of script running — which is the whole reason
  // the class and the depth are all this writes.
  const root = doc.getElementById('landing-panel');
  if (root) {
    // The panel's own show/hide, for the surface that asked to own it.
    // Written before anything else so a frame that both dismisses the landing
    // and rewrites its text paints once.
    if (ownPanelVisibility) root.style.display = vm.stage === 'dismissed' ? 'none' : '';
    // `rootClass` may name more than one class — `is-open is-deep` is the
    // staged New Game (issue #1362) — so it is split rather than added whole:
    // `classList.add` throws on a string with a space in it. Everything this
    // renderer can write is removed first, so a stage that no longer applies
    // cannot be left behind by a shorter one.
    root.classList.remove(...LANDING_ROOT_CLASSES);
    String(vm.rootClass || '').split(/\s+/).forEach(function (name) {
      if (name) root.classList.add(name);
    });
    root.dataset.landingStage = vm.stage;
    root.style.setProperty('--landing-depth', String(vm.depth));
  }

  // ── Identity ────────────────────────────────────────────────────────
  setText(doc, 'landing-title', t(vm.identity.titleId));
  setText(doc, 'landing-tagline', t(vm.identity.taglineId));
  setText(doc, 'landing-platform', t(vm.identity.platformLabelId));
  const logo = doc.getElementById('landing-logo');
  // The image itself is a background in the stylesheet (a URL in a stylesheet
  // resolves against the STYLESHEET, which is what lets one sheet serve two
  // documents at two depths — see gui/host-landing.css). What is left here is
  // the accessible name, which is text and therefore localised.
  if (logo) logo.setAttribute('aria-label', t(vm.identity.logoAltId));

  // ── Status bar ──────────────────────────────────────────────────────
  setText(doc, 'landing-status-platform', t(vm.status.platformLabelId));
  setText(doc, 'landing-status-session', t(vm.status.sessionId));
  setText(doc, 'landing-status-build', t(vm.status.build.id, vm.status.build.params));

  // ── The corner fullscreen control ───────────────────────────────────
  //
  // `onclick` rather than addEventListener, deliberately: this button is
  // STATIC markup and this function runs again on every menu click, so an
  // added listener would accumulate one toggle per render and the third click
  // would flip fullscreen three times. Assignment is idempotent.
  const fs = doc.getElementById('landing-fullscreen-btn');
  if (fs) fs.onclick = h.toggleFullscreen || null;

  // ── The menu ────────────────────────────────────────────────────────
  const menu = doc.getElementById('landing-menu');
  if (menu) {
    // Which entry the operator is standing on, if any — captured as an ID
    // rather than as the node, because the node is about to be deleted.
    //
    // This render runs on EVERY click of every entry, so without this a
    // keyboard or gamepad operator loses their place by activating the very
    // control they had reached: the focused button is removed, focus falls to
    // `<body>`, and the next Tab restarts at the top of the document. The
    // landing is the surface most likely to be driven entirely by keys — it is
    // the front door, before any pointer-friendly stage is open.
    const active = doc.activeElement;
    const refocusId = active && active.getAttribute
      ? active.getAttribute(LANDING_ENTRY_ATTR)
      : null;
    let refocus = null;

    clearLandingEntries(menu);
    vm.entries.forEach(function (entry) {
      const btn = doc.createElement('button');
      btn.type = 'button';
      btn.className = 'landing-mi ' + LANDING_ENTRY_CLASS
        + (entry.selected ? ' on' : '')
        + (entry.inert ? ' inert' : '');
      btn.setAttribute(LANDING_ENTRY_ATTR, entry.id);
      // An entry with a stage is a disclosure; one without is a control that
      // does not do anything yet. `aria-disabled` rather than `disabled`
      // because it stays focusable and readable — a control a screen reader
      // cannot reach is not more honest than one that says it is unavailable.
      if (entry.inert) btn.setAttribute('aria-disabled', 'true');
      else btn.setAttribute('aria-expanded', entry.selected ? 'true' : 'false');

      const bar = doc.createElement('span');
      bar.className = 'landing-mi-bar';
      btn.appendChild(bar);

      const ix = doc.createElement('span');
      ix.className = 'landing-mi-ix';
      ix.textContent = entry.ordinal;
      btn.appendChild(ix);

      const body = doc.createElement('span');
      body.className = 'landing-mi-body';
      const label = doc.createElement('span');
      label.className = 'landing-mi-label';
      label.textContent = t(entry.labelId);
      const desc = doc.createElement('span');
      desc.className = 'landing-mi-desc';
      desc.textContent = t(entry.descId);
      body.appendChild(label);
      body.appendChild(desc);
      btn.appendChild(body);

      btn.addEventListener('click', function () {
        if (h.pick) h.pick(entry.id);
      });
      if (refocusId && entry.id === refocusId) refocus = btn;
      menu.appendChild(btn);
    });

    // `doc.activeElement === doc.body` is the whole condition: it means the
    // rebuild above is what dropped focus, and nothing else has claimed it
    // since. A render that happens while the operator is somewhere else — in
    // the docked World picker, in the settings overlay — leaves that alone
    // rather than yanking them back to the menu. `focus` is guarded for the
    // same reason every write here is: a surface may hand us a document whose
    // elements are not a full HTMLElement implementation.
    if (refocus && doc.activeElement === doc.body && typeof refocus.focus === 'function') {
      refocus.focus();
    }

    // ── A receding column is an INACTIVE one, not a faint one ──────────
    //
    // `gui/host-landing.css` dims this column to 42% one rung deep, which is
    // the design's recession and is also far under the 4.5:1 floor the same
    // sheet holds these labels to at rest. The contrast floor exempts exactly
    // one kind of text — text in an inactive user-interface component — so
    // the column is made inactive rather than left as live controls nobody
    // can read: the sheet drops the pointer, this drops the keyboard. Back is
    // the documented way out of a deep stage, so the menu is not the exit
    // being closed off.
    //
    // Written from the DEPTH, the same fact the stylesheet slides by, rather
    // than from the name `is-deep`: a route that grows a third rung is still
    // "deeper than its first column" without this module learning a name.
    // Written after the rebuild so the focus carry above is never asked to
    // land on a node this line has just made unfocusable.
    if (typeof menu.setAttribute === 'function') {
      if (vm.depth > 1) menu.setAttribute('inert', '');
      else menu.removeAttribute('inert');
    }
  }

  // ── The middle column ───────────────────────────────────────────────
  //
  // "Reveals the EXISTING World picker" is meant literally: `#scenario-panel`
  // is moved into `#landing-mid` rather than re-rendered here, so a pick still
  // reaches `driveWorldLoad()` down the path it always did, and the mod-pack
  // upload, the save importer and the docked join panel travel with it because
  // they are its children.
  //
  // Undocking appends it back to `<body>`, which is a restoration and not an
  // approximation of one: the panel is `position: fixed; inset: 0` with an
  // explicit `z-index: 200`, so where it sits among the body's children
  // decides neither its box nor its paint order. That is what lets this be
  // stateless — no remembered parent to go stale between two documents.
  //
  // The condition is `vm.docksPicker`, a FACT ABOUT THE ROW, and not a
  // comparison of `vm.stage` to 'world-picker' (issue #1362). The staged New
  // Game is two stages deep and the picker has to stay put across both — a
  // stage-name test would have undocked it the moment the hulls appeared,
  // taking the World list with it. It also keeps the rule readable by a future
  // entry: an entry that borrows this panel says so on its own line.
  if (dockPicker) {
    const mid = doc.getElementById('landing-mid');
    const picker = doc.getElementById('scenario-panel');
    if (mid && picker) {
      if (vm.docksPicker) {
        if (picker.parentElement !== mid) mid.appendChild(picker);
        picker.classList.add(PICKER_DOCKED_CLASS);
      } else if (picker.parentElement === mid) {
        picker.classList.remove(PICKER_DOCKED_CLASS);
        if (doc.body) doc.body.appendChild(picker);
      }
    }
  }
}

/**
 * Put `#scenario-panel` back where it belongs, whatever the landing is doing.
 *
 * The page lifecycle's own escape hatch: `driveWorldLoad()` hides the landing
 * outright the moment a world is chosen, and the picker must not be left
 * parented inside a hidden panel — the second round after a Game Over re-shows
 * `#scenario-panel` by setting its `display`, and a `display` on a node inside
 * a hidden ancestor shows nothing at all.
 */
export function undockPicker(doc) {
  const picker = doc.getElementById('scenario-panel');
  if (!picker) return;
  picker.classList.remove(PICKER_DOCKED_CLASS);
  const mid = doc.getElementById('landing-mid');
  if (mid && picker.parentElement === mid && doc.body) doc.body.appendChild(picker);
}

// Expose for the classic (non-module) script in server.html — the same
// self-registering pattern window.hostScenarioRender uses.
if (typeof window !== 'undefined') {
  window.hostLandingRender = {
    renderHostLanding,
    undockPicker,
    clearLandingEntries,
    LANDING_ENTRY_CLASS,
    LANDING_ENTRY_SELECTOR,
    LANDING_ENTRY_ATTR,
    PICKER_DOCKED_CLASS,
    LANDING_ROOT_CLASSES,
  };
}
