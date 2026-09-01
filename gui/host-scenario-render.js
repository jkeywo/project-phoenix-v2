/**
 * gui/host-scenario-render.js — the scenario picker's renderer (issue #1328).
 *
 * The DOM half of the pair whose pure half is `gui/host-scenarios.js`:
 * `scenarioCatalogView()` decides which stage of the QR-first picker to show,
 * this writes it into a document. Both used to live inside `server.html`'s
 * `renderScenarioLockState()` — #1230 lifted out the decision, this lifts out
 * the writes — because the NATIVE host now shows the same picker on its
 * viewscreen window (`src/native_host/host_lobby/`), and a second
 * implementation of `#world-list`, `.scenario-entry` and `ph-ship-picker` is a
 * second picker that drifts from this one the first time either is touched.
 *
 * It is the exact sibling of `gui/host-lobby-render.js` (issue #1325), which
 * did this for the crew lobby, and it is read the same way: no state of its
 * own, no transport, no arbiter. Given a view model it writes a document, and
 * everything it cannot do itself it asks the caller for.
 *
 * ## What it writes, and what it deliberately does not
 *
 * Exactly the contents of `#world-list` that the picker owns: the column
 * label, the scenario buttons, the `ph-ship-picker`, and the
 * `#scenario-loading` placeholder that stands in for both "still loading" and
 * "this manifest publishes nothing".
 *
 * It does NOT touch anything else in `#scenario-panel`. The mod-pack upload
 * and the save importer are static markup with page-lifetime handlers and a
 * demo-build removal rule of their own, which is why `SCENARIO_ENTRY_SELECTOR`
 * is `.scenario-entry` and not `.world-btn` — the latter is a shared *styling*
 * hook those two buttons also wear, and cleaning up by it deleted the "Upload
 * mod pack" button the instant the first scenario button rendered (issue #951).
 *
 * ## The side effects are hooks, because they are not the same on both surfaces
 *
 * A pick has to reach an arbiter, and the two surfaces reach different ones:
 * `server.html` holds the arbiter in its own page (`gui/scenario-arbiter.js`,
 * driven by `arbiterSelectScenario`/`arbiterSelectShip`), while the native
 * surface has no arbiter at all — the host process does, in
 * `src/lobby/scenario_arbiter.rs`, so its picks travel back over the host-lobby
 * bridge as records. Neither is this module's business, so both arrive as
 * `hooks`.
 *
 * `t` is passed in for the reason `renderHostLobby` takes it: `server.html`
 * holds a classic-script `t()` closed over `window.phStrings`, and the native
 * lobby document imports `gui/strings.js` directly.
 */

/** Lifecycle class worn by every element this renderer creates. */
export const SCENARIO_ENTRY_CLASS = 'scenario-entry';

/**
 * Everything this renderer OWNS and rebuilds on every call.
 *
 * Deliberately NOT `.world-btn`: that is a shared *styling* hook also worn by
 * the static `#mod-pack-btn` and `#snapshot-import-btn`, so cleaning up by it
 * deleted the "Upload mod pack" button the instant the first scenario button
 * rendered (issue #951). Styling hook and lifecycle hook are separate on
 * purpose — anything added to `#world-list` that this renderer does not own
 * must stay out of this selector.
 *
 * (`#scenario-loading` is matched by id by design: it is the placeholder this
 * renderer replaces.)
 */
export const SCENARIO_ENTRY_SELECTOR = '.scenario-entry, ph-ship-picker, #scenario-loading';

/** Remove every element this renderer owns, leaving static controls alone. */
export function clearScenarioEntries(worldList) {
  worldList.querySelectorAll(SCENARIO_ENTRY_SELECTOR).forEach(function (el) { el.remove(); });
}

/**
 * Insert a rebuilt entry above the static footer controls so
 * `#mod-pack-upload` keeps the bottom of the column its `margin-top: auto`
 * asks for (plain appendChild would stack scenario buttons below the upload
 * block's separator rule).
 *
 * On the native lobby surface there is no footer — the document strips the
 * host tooling — and the `else` branch is the whole of that case.
 */
export function appendScenarioEntry(worldList, el) {
  const footer = worldList.querySelector('#mod-pack-upload');
  if (footer) worldList.insertBefore(el, footer);
  else worldList.appendChild(el);
}

/**
 * Render one scenario-picker view model into `doc`.
 *
 * @param {Document} doc the document holding the `#scenario-panel` markup.
 * @param {object} vm the return of `scenarioCatalogView()`.
 * @param {(id: string, params?: object) => string} t string-id resolver.
 * @param {{
 *   tData?: (value: *) => string,
 *   selectScenario?: (scenarioId: string) => void,
 *   selectShip?: (templatePath: string) => void,
 *   autoSelectShip?: (templatePath: string) => void,
 *   shipStillNeeded?: () => boolean,
 * }} [hooks]
 *   `tData` resolves authored data labels (a world's `[[available_ships]]
 *   label` is a string id, not prose — issue #949); `selectScenario` /
 *   `selectShip` carry an operator's click to whichever arbiter this surface
 *   reaches; `autoSelectShip` is the single-hull auto-resolve (issue #917),
 *   separate because `server.html` answers it by calling its arbiter DIRECTLY
 *   while a click goes through its action map, and collapsing the two would
 *   change which path the web host takes; `shipStillNeeded` is re-checked
 *   after the `ph-ship-picker` module has loaded, because that load is async
 *   and another participant may have locked the hull while it was in flight.
 * @param {{ownPanelVisibility?: boolean}} [opts] `ownPanelVisibility` makes
 *   this renderer show and hide `#scenario-panel` itself. It is the native
 *   lobby surface's option (issue #1328) and is the exact sibling of
 *   `renderHostLobby`'s `revealChrome`: on the host PAGE the panel's
 *   visibility is page lifecycle (`driveWorldLoad` hides it, the return to
 *   lobby shows it again) and is not driven by this render at all, so
 *   `server.html` passes nothing and gets the behaviour it always had.
 */
export function renderHostScenarios(doc, vm, t, hooks, opts) {
  const h = hooks || {};
  const tData = h.tData || ((value) => (value == null ? '' : String(value)));
  const ownPanelVisibility = !!(opts && opts.ownPanelVisibility);

  const worldList = doc.getElementById('world-list');
  const label = doc.getElementById('world-list-label');
  if (!worldList) return;

  // The panel's own show/hide, for the surface that asked to own it. Written
  // before anything else so a frame that both hides the panel and clears it
  // paints once.
  if (ownPanelVisibility) {
    const panel = doc.getElementById('scenario-panel');
    if (panel) panel.style.display = vm.stage === 'locked' ? 'none' : '';
  }

  if (vm.stage === 'ship-auto') {
    // A scenario curated (or authored) down to exactly one playable hull
    // resolves straight to it — no picker click needed (issue #917). Nothing
    // is rendered: the caller's arbiter answers, and the answer re-renders.
    const auto = h.autoSelectShip || h.selectShip;
    if (auto) auto(vm.templatePath);
    return;
  }

  if (vm.stage === 'scenario-empty') {
    if (label) label.textContent = t(vm.labelId);
    clearScenarioEntries(worldList);
    const empty = doc.createElement('div');
    empty.id = 'scenario-loading';
    empty.textContent = t('server.no_scenarios');
    appendScenarioEntry(worldList, empty);
    return;
  }

  if (vm.stage === 'scenario-list') {
    // Scenario stage — one button per catalog scenario.
    if (label) label.textContent = t(vm.labelId);
    clearScenarioEntries(worldList);
    vm.entries.forEach(function (sc) {
      const btn = doc.createElement('button');
      btn.className = 'world-btn ' + SCENARIO_ENTRY_CLASS;
      btn.dataset.path = sc.world;
      btn.dataset.scenarioId = sc.scenarioId;
      btn.textContent = tData(sc.label) || sc.scenarioId;
      btn.addEventListener('click', function () {
        if (h.selectScenario) h.selectScenario(sc.scenarioId);
      });
      appendScenarioEntry(worldList, btn);
    });
    return;
  }

  if (vm.stage === 'ship-picker') {
    // Ship stage — the locked scenario's offered hulls, via ph-ship-picker.
    // The specifier is relative to THIS module rather than to the page, which
    // is what lets two documents at two depths load one component.
    if (label) label.textContent = t(vm.labelId);
    clearScenarioEntries(worldList);
    import('./components/ph-ship-picker.js').then(function () {
      // Won meanwhile: the import is async and the arbiter is first-valid-wins,
      // so a phone (or the host's own second click) may have locked the hull
      // while the component was loading.
      if (h.shipStillNeeded && !h.shipStillNeeded()) return;
      const picker = doc.createElement('ph-ship-picker');
      appendScenarioEntry(worldList, picker);
      picker.state = { ships: vm.ships };
      picker.addEventListener('ship-selected', function (e) {
        if (h.selectShip) h.selectShip(e.detail.template_path);
      });
    });
    return;
  }

  // vm.stage === 'locked' — either the world load has started, or both scenario
  // and hull are locked and it is about to. Only this render's own entries go;
  // the static footer controls stay put for the next round's scenario stage
  // (Return to Lobby re-shows the panel without rebuilding it).
  clearScenarioEntries(worldList);
}

// Expose for the classic (non-module) script in server.html — the same
// self-registering pattern window.hostLobbyRender uses.
if (typeof window !== 'undefined') {
  window.hostScenarioRender = {
    renderHostScenarios,
    clearScenarioEntries,
    appendScenarioEntry,
    SCENARIO_ENTRY_CLASS,
    SCENARIO_ENTRY_SELECTOR,
  };
}
