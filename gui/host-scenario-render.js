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
 * Since issue #1362 it also owns `#ship-list` — a SECOND column, on a surface
 * that carries one. That is the whole of the staged layout's mechanism here:
 * the hulls go into their own column and the World rows stay in theirs, so
 * choosing a World reveals the hulls BESIDE the list rather than in place of
 * it. See `SHIP_LIST_ID` for why the column is found by id rather than passed
 * as an option, and the `ship-picker` branch for what each surface gets.
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
 *
 * ## The column is rebuilt, so focus is carried across the rebuild
 *
 * Every stage below replaces every control it owns, and Back's own click is
 * what re-renders — so without help the operator's activation of a control
 * destroys the node they were standing on, focus falls to `<body>`, and the
 * next Tab restarts at the top of the document. `gui/host-landing-render.js`
 * carries the menu across its rebuild for the same reason and states the case
 * at length. Here it is Back that earns it, because Back is a step BACKWARDS
 * and the place to put the operator is known: the World row they released,
 * whose id rides on the Back control itself.
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

/**
 * The column a surface offers for the HULL stage, when it has one (issue #1362).
 *
 * Looked up by id and simply absent on a surface that does not carry it, which
 * is this module's usual two-document guard rather than a new option threaded
 * through every caller. A document WITH it gets the design's staged layout —
 * the hulls beside the World rows that led to them — and a document without it
 * gets what the picker always did: the hull cards in the world column, under
 * the rows rather than instead of them.
 */
export const SHIP_LIST_ID = 'ship-list';

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
 *   backToWorlds?: () => void,
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
 *
 *   `backToWorlds` steps out of the hull stage (issue #1362). It is a hook and
 *   not a branch here for the reason every other side effect is: releasing a
 *   locked World is an ARBITER move, and the two surfaces reach two different
 *   arbiters — the page's own on the web, the host process's on native. A
 *   surface that supplies none gets no Back control at all, which is the
 *   honest rendering of "there is nothing behind it".
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
  // The hull column, on a surface that carries one (issue #1362). Absent is a
  // supported document rather than a broken one — see SHIP_LIST_ID.
  const shipList = doc.getElementById(SHIP_LIST_ID);
  const shipLabel = doc.getElementById('ship-list-label');
  if (!worldList) return;

  // ── Where the operator is standing, captured before this render moves it ──
  //
  // Every branch below rebuilds `#world-list` from scratch and empties the
  // hull column, so an operator on the keyboard loses their place by
  // activating the very control they had reached: the focused node is removed,
  // focus falls to `<body>`, and the next Tab restarts at the top of the
  // document. `gui/host-landing-render.js` solves exactly this for the menu it
  // rebuilds, and says why in more detail — this is that pattern on the column
  // beside it (issue #1362).
  //
  // Back is the case worth the code, because it is a step BACKWARDS: the place
  // to put the operator is KNOWN — the World row they just released — where a
  // forward click leaves them somewhere that does not exist yet. The row's id
  // travels on the Back control itself (see `back.dataset.scenarioId`), so
  // this stays as stateless as the rest of the module: no remembered previous
  // view model, just the document saying what it was showing a moment ago.
  const wasFocused = doc.activeElement;
  const backHadFocus = !!(wasFocused && wasFocused.classList
    && wasFocused.classList.contains('scenario-back'));
  const backFromScenarioId = (backHadFocus && wasFocused.dataset)
    ? (wasFocused.dataset.scenarioId || null)
    : null;
  // Claimed as the rows are built, so the match is made once rather than by
  // re-querying the column this function has just written.
  let refocusRow = null;
  let firstRow = null;

  /**
   * Hand focus back to the World row the operator released, and only when this
   * render is what took it.
   *
   * `doc.activeElement === doc.body` is the whole condition, for the reason
   * the landing renderer gives it: it means the rebuild above dropped focus
   * and nothing else has claimed it since, so a render that lands while the
   * operator is somewhere else does not yank them back to the list.
   */
  function restoreFocusAfterBack(target) {
    if (!backHadFocus) return;
    if (doc.activeElement !== doc.body) return;
    if (target && typeof target.focus === 'function') target.focus();
  }

  /** One World row, for whichever of the two stages is drawing the column. */
  function worldButton(sc) {
    const btn = doc.createElement('button');
    btn.className = 'world-btn ' + SCENARIO_ENTRY_CLASS + (sc.selected ? ' active' : '');
    btn.dataset.path = sc.world;
    btn.dataset.scenarioId = sc.scenarioId;
    // The row's own name, in a span of its own so the hull count beside it is
    // a sibling and not part of the title.
    const name = doc.createElement('span');
    name.className = 'world-btn-name';
    name.textContent = tData(sc.label) || sc.scenarioId;
    btn.appendChild(name);
    // How many hulls this World offers, BEFORE it is chosen (issue #1362).
    // Null means the World publishes no curated list, which reads as
    // unrestricted rather than as none — so nothing is drawn instead of a "0"
    // that would be the one wrong answer. `hullCountLabel` is an {id, params}
    // pair; the count decided which id, the string table decides the words.
    if (sc.hullCountLabel) {
      const chip = doc.createElement('span');
      chip.className = 'world-btn-hulls' + (sc.hasChoice ? ' on' : '');
      chip.textContent = t(sc.hullCountLabel.id, sc.hullCountLabel.params);
      btn.appendChild(chip);
    }
    // The row an operator is standing in, for a reader who cannot see the
    // accent. Set only on the chosen one, never as `aria-current="false"`.
    if (sc.selected) btn.setAttribute('aria-current', 'true');
    btn.addEventListener('click', function () {
      if (h.selectScenario) h.selectScenario(sc.scenarioId);
    });
    if (!firstRow) firstRow = btn;
    if (backFromScenarioId != null && sc.scenarioId === backFromScenarioId) refocusRow = btn;
    return btn;
  }

  // The hull column is rebuilt by every stage, not only by the one that fills
  // it: a Back out of the hulls, a world load, or another participant winning
  // the pick all leave this stage, and a column still holding last stage's
  // cards is a control an operator can still press.
  if (shipList) clearScenarioEntries(shipList);

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
      appendScenarioEntry(worldList, worldButton(sc));
    });
    // The stage a Back lands on. `firstRow` is the fallback for a Back that
    // carried no id — a surface whose view model does not publish one — and is
    // the top of the list, which is where a reader starts anyway.
    restoreFocusAfterBack(refocusRow || firstRow);
    return;
  }

  if (vm.stage === 'ship-picker') {
    // Ship stage — the locked scenario's offered hulls, via ph-ship-picker.
    // The specifier is relative to THIS module rather than to the page, which
    // is what lets two documents at two depths load one component.
    //
    // WHERE the hulls go is the document's answer, not an option a caller has
    // to remember to pass. A surface carrying `#ship-list` gets the design's
    // staged layout — the hulls in their own column, the World rows still
    // standing in theirs with the chosen one marked, so the operator can see
    // the path they took and step back along it (issue #1362). A surface
    // without it keeps what the picker always did: the world column becomes
    // the hull column. That is not a lesser rendering of the same idea, it is
    // the only honest one on a document with one column to draw in.
    const host = shipList || worldList;
    const beside = !!shipList;
    if (beside) {
      if (label) label.textContent = t(vm.worldLabelId || 'server.select_world');
      if (shipLabel) shipLabel.textContent = t(vm.labelId);
      clearScenarioEntries(worldList);
      vm.entries.forEach(function (sc) {
        appendScenarioEntry(worldList, worldButton(sc));
      });
    } else {
      if (label) label.textContent = t(vm.labelId);
      clearScenarioEntries(worldList);
    }

    // The step back out of the hull stage, drawn only when a surface has an
    // arbiter to answer it. Appended BEFORE the picker exists so the column's
    // order does not depend on how long the dynamic import took; the card grid
    // is inserted above it when it lands.
    let back = null;
    if (h.backToWorlds) {
      back = doc.createElement('button');
      back.type = 'button';
      back.className = 'scenario-back ' + SCENARIO_ENTRY_CLASS;
      back.textContent = t('server.back_to_worlds');
      // Where back goes, carried on the control that goes there. This is the
      // whole of how the next render knows which World row to hand focus to,
      // and it is written on the DOM rather than remembered in a module
      // variable for the reason nothing else here is remembered: two surfaces
      // call this function over two documents, and a module-level memory would
      // be one document answering for the other.
      if (vm.scenarioId) back.dataset.scenarioId = vm.scenarioId;
      back.addEventListener('click', function () { h.backToWorlds(); });
      if (beside) host.appendChild(back);
      else appendScenarioEntry(host, back);
      // A re-render of the hull stage itself — a phone locking a hull, a
      // catalog rebuild — deletes and recreates this control too, so the
      // operator standing on it is put back on the one that replaced it.
      restoreFocusAfterBack(back);
    }

    import('./components/ph-ship-picker.js').then(function () {
      // Won meanwhile: the import is async and the arbiter is first-valid-wins,
      // so a phone (or the host's own second click) may have locked the hull
      // while the component was loading.
      if (h.shipStillNeeded && !h.shipStillNeeded()) return;
      // Gone meanwhile: a Back (or a world load) between this render and the
      // import landing has already emptied the column, and re-parenting a
      // detached Back button would resurrect the stage the operator just left.
      if (back && !back.parentNode) return;
      const picker = doc.createElement('ph-ship-picker');
      if (back) host.insertBefore(picker, back);
      else if (beside) host.appendChild(picker);
      else appendScenarioEntry(host, picker);
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
    SHIP_LIST_ID,
  };
}
