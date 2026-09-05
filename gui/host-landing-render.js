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
 * lobby document carries a trimmed subset of this markup, so a branch whose
 * element is absent must do nothing rather than throw and abandon the rest of
 * the render half-written. The suite drives a deliberately incomplete document
 * for exactly this. It stopped being only a hypothetical in #1367: the native
 * document really did have this landing's fullscreen control cut out of it
 * until that slice gave the control a host verb to reach.
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
 * surfaces own it in different places. Fullscreen is the clearest case for a
 * hook rather than a call, because the two surfaces do not even mean the same
 * thing by it: on the web it is already implemented once, in
 * `gui/page-chrome.js`'s `initFullscreen`, and the corner control forwards to
 * that; on the native viewscreen there is no browser to ask, so the same
 * control sends `toggle_fullscreen` over the page->host queue and the host
 * moves the window's mode (issue #1367). This module knows neither, which is
 * the only reason one control can serve both.
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
 * ## The join field's value is the operator's, not the view model's
 *
 * The join-code stage (issue #1364) is written from `vm.join` like everything
 * else here, with one exception that is deliberate: this never writes
 * `#landing-join-code`'s value. Every render is a rebuild, and a rebuild that
 * restored the value would fight somebody mid-word while one that cleared it
 * would delete the eight letters a refusal is about. The sentences around the
 * field are this module's; what is in it is theirs.
 *
 * ## What it does NOT draw
 *
 * The settings cog — on either surface, and that is one rule rather than a gap
 * on each. A settings cog is the settings overlay's OWN control: it is
 * find-or-created by `gui/settings-overlay-kit.js` beside the modal it opens,
 * so it exists exactly where an overlay has been mounted and cannot get out of
 * step with it. `gui/server-settings.js` mounts `#server-settings-btn` on the
 * host page (issue #939) and `gui/native-settings.js` mounts
 * `#native-settings-btn` on the viewscreen (issue #1367); both are `fixed`
 * above this panel and both land in the corner the design draws the cog in,
 * which is inside `.landing-rail`. Drawing one here as well would be two
 * controls disagreeing about which panel is open.
 *
 * Nor does the landing reserve a keep-out for it the way `#world-list` does in
 * `gui/host-scenarios.css`: the rail's content sits at the far end and
 * everything else starts below the cog — the clearance is layout, and
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

/**
 * The class a borrowed panel wears while it is docked in the middle column.
 *
 * One class for both borrowers — `#scenario-panel` (issue #1362) and
 * `#save-slots-panel` (issue #1363) — because it is what the STYLESHEET keys
 * the unwinding off, and each panel names itself in its own selector there.
 * It is also how an undock finds what to put back without this module keeping
 * a list of dockable ids: whatever is parked in the column wearing this class
 * was parked by this renderer.
 */
export const LANDING_DOCKED_CLASS = 'landing-docked';

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

/**
 * The class `#landing-confirm` wears while it is asking about something
 * destructive (issue #1365).
 *
 * Driven off the open row's `confirm.tone`, never off its id: which
 * confirmations are destructive is the entry table's knowledge, and a renderer
 * deciding it from an id would be the second place that decision lived.
 */
export const CONFIRM_DANGER_CLASS = 'landing-confirm-danger';

/**
 * Everything this renderer OWNS inside `#landing-packs-list` (issue #1366).
 *
 * A lifecycle hook for the reason [`LANDING_ENTRY_SELECTOR`] is one: this
 * renderer replaces every shelf row on every render, and a selector that also
 * matched static markup would delete controls it does not own.
 */
export const LANDING_PACK_SELECTOR = '.landing-pack';

/** The attribute a shelf row carries its archive's file name in. */
export const LANDING_PACK_ATTR = 'data-landing-pack';

/**
 * Everything this renderer owns inside `#landing-packs-notes` — the outcome
 * line, the findings, the installed list and the conflict report.
 *
 * One selector for all four because they are one rebuilt column: every render
 * replaces the lot, and what separates them is the heading each carries rather
 * than a container each would need.
 */
export const LANDING_NOTE_SELECTOR = '.landing-note';

/** Remove every entry this renderer owns, leaving anything else alone. */
export function clearLandingEntries(menu) {
  menu.querySelectorAll(LANDING_ENTRY_SELECTOR).forEach(function (el) { el.remove(); });
}

/** Remove every element matching `selector` inside `root`. */
function clearOwned(root, selector) {
  root.querySelectorAll(selector).forEach(function (el) { el.remove(); });
}

/** Write `text` into `id` if this document has it. */
function setText(doc, id, text) {
  const el = doc.getElementById(id);
  if (el) el.textContent = text;
}

/** Write `t(id)` into `elementId`, or clear it when the row named no string. */
function setOptionalText(doc, elementId, t, id) {
  setText(doc, elementId, id ? t(id) : '');
}

/**
 * Draw `#landing-confirm` from the open row's confirmation block (issue #1365).
 *
 * The stage the menu's one irreversible route opens. Everything it says comes
 * off `vm.confirm`, which is the ROW's block republished — so this function
 * knows there is such a thing as a confirmation and knows nothing whatever
 * about Exit to Desktop. A second confirming entry is a second row.
 *
 * Two things are worth stating rather than reading back out of the code:
 *
 *   * **Cancel is the entry's own toggle.** It reports through `pick` with the
 *     open entry's id, which is exactly what a second press on the menu entry
 *     does, and `nextOpenEntry` closes it. A `cancel` hook of its own would be
 *     a second way to close one stage, and the day the two disagreed the
 *     operator would be the one to find out.
 *   * **`onclick`, not `addEventListener`.** These are STATIC markup and this
 *     function runs again on every render, so an added listener would
 *     accumulate one quit per render — see the fullscreen control's note.
 */
function renderConfirm(doc, vm, t, h) {
  const confirm = vm.confirm || null;
  const root = doc.getElementById('landing-confirm');
  if (root) {
    root.style.display = confirm ? '' : 'none';
    root.classList.toggle(CONFIRM_DANGER_CLASS, !!confirm && confirm.tone === 'danger');
  }
  // The writes below happen whether or not the panel is on screen — the same
  // reason the menu is rebuilt every render: a hidden panel holding the last
  // route's words is a panel that shows them for one frame the next time it
  // opens. `confirm` being null clears rather than skips.
  setOptionalText(doc, 'landing-confirm-title', t, confirm && confirm.titleId);
  setOptionalText(doc, 'landing-confirm-eyebrow', t, confirm && confirm.eyebrowId);
  setOptionalText(doc, 'landing-confirm-lead', t, confirm && confirm.leadId);
  setOptionalText(doc, 'landing-confirm-note', t, confirm && confirm.noteId);
  setOptionalText(doc, 'landing-confirm-cta', t, confirm && confirm.ctaId);
  setOptionalText(doc, 'landing-confirm-cancel', t, confirm && confirm.cancelId);

  const cta = doc.getElementById('landing-confirm-cta');
  if (cta) {
    cta.classList.toggle(CONFIRM_DANGER_CLASS, !!confirm && confirm.tone === 'danger');
    cta.onclick = confirm && h.confirm
      ? function () { h.confirm(confirm.action); }
      : null;
  }
  const cancel = doc.getElementById('landing-confirm-cancel');
  if (cancel) {
    cancel.onclick = confirm && h.pick
      ? function () { h.pick(vm.openEntryId); }
      : null;
  }
}

/**
 * One note row in the shelf's report column: a label, a line, and a tone.
 *
 * `line` is either a resolved sentence (a validator's prose) or a `{id, params}`
 * pair — the two shapes `vm.packs` deliberately keeps apart, and the reason this
 * takes text rather than an id.
 */
function appendNote(doc, root, tone, label, text) {
  const note = doc.createElement('div');
  note.className = 'landing-note' + (tone ? ' landing-note-' + tone : '');
  if (label) {
    const key = doc.createElement('span');
    key.className = 'landing-note-key';
    key.textContent = label;
    note.appendChild(key);
  }
  const value = doc.createElement('span');
  value.className = 'landing-note-value';
  value.textContent = text;
  note.appendChild(value);
  root.appendChild(note);
}

/**
 * Draw `#landing-packs` from the open row's shelf (issue #1366).
 *
 * The middle column's third tenant, and the exact sibling of
 * [`renderConfirm`]: everything it says comes off `vm.packs`, which the view
 * model composes only while the row that NEEDS a shelf is the open one — so
 * this function knows there is such a thing as a shelf and knows nothing at all
 * about Load mod pack. A second folder-shaped stage would be a second row.
 *
 * Three things worth stating rather than reading back out of the code:
 *
 *   * **The rows are rebuilt, and so is the report column.** A hidden panel
 *     holding the last folder's archives is a panel that shows them for one
 *     frame the next time it opens — the same reason the menu is rebuilt.
 *   * **Cancel is the entry's own toggle**, reported through `pick` with the
 *     open entry's id, exactly as the confirmation's is and for the same
 *     reason: one way to close one stage.
 *   * **The CTA carries the ROW's verb**, not a name this file knows. It is
 *     handed to `installPack` with the highlighted file, and what that means is
 *     the caller's business.
 */
function renderPacks(doc, vm, t, h) {
  const packs = vm.packs || null;
  const root = doc.getElementById('landing-packs');
  if (root) root.style.display = packs ? '' : 'none';

  setOptionalText(doc, 'landing-packs-title', t, packs && packs.titleId);
  setText(doc, 'landing-packs-folder', packs ? t(packs.folder.id, packs.folder.params) : '');

  // The empty state carries BOTH halves when there are both: the id says which
  // emptiness this is, and the host's own sentence names the folder it could
  // not read. A scan failure with no sentence still says something.
  const empty = doc.getElementById('landing-packs-empty');
  if (empty) {
    const words = packs && packs.emptyId
      ? [t(packs.emptyId), packs.scanError || ''].filter(Boolean).join(' ')
      : '';
    empty.textContent = words;
    empty.style.display = words ? '' : 'none';
  }

  const list = doc.getElementById('landing-packs-list');
  if (list) {
    clearOwned(list, LANDING_PACK_SELECTOR);
    (packs ? packs.rows : []).forEach(function (pack) {
      const btn = doc.createElement('button');
      btn.type = 'button';
      btn.className = 'landing-pack' + (pack.selected ? ' on' : '');
      btn.setAttribute(LANDING_PACK_ATTR, pack.file);
      btn.setAttribute('aria-pressed', pack.selected ? 'true' : 'false');
      const label = doc.createElement('span');
      label.className = 'landing-pack-label';
      label.textContent = pack.label;
      btn.appendChild(label);
      const file = doc.createElement('span');
      file.className = 'landing-pack-file';
      file.textContent = pack.file;
      btn.appendChild(file);
      btn.addEventListener('click', function () {
        if (h.pickPack) h.pickPack(pack.file);
      });
      list.appendChild(btn);
    });
  }

  const notes = doc.getElementById('landing-packs-notes');
  if (notes) {
    clearOwned(notes, LANDING_NOTE_SELECTOR);
    if (packs) {
      if (packs.outcome) {
        appendNote(doc, notes, packs.outcome.tone, '',
          t(packs.outcome.line.id, packs.outcome.line.params));
      }
      // What is wrong, in the validator's own words. This is the acceptance
      // criterion "a pack that fails validation says what is wrong", and it is
      // drawn whether the attempt was accepted or not — a warning-only accept
      // still has something to say.
      packs.findings.forEach(function (finding) {
        appendNote(doc, notes, finding.tone, t(finding.labelId),
          [finding.file, finding.message].filter(Boolean).join(' — '));
      });
      if (packs.installedHeadingId) {
        appendNote(doc, notes, 'head', '', t(packs.installedHeadingId));
      }
      packs.installed.forEach(function (pack) {
        appendNote(doc, notes, 'ok', '', t(pack.line.id, pack.line.params));
      });
      // Which pack won each shared path. The acceptance criterion "conflicting
      // packs are reported, so it is clear which pack won".
      if (packs.conflictsHeadingId) {
        appendNote(doc, notes, 'head', '', t(packs.conflictsHeadingId));
      }
      packs.conflicts.forEach(function (conflict) {
        appendNote(doc, notes, 'warn', '', t(conflict.line.id, conflict.line.params));
      });
    }
  }

  // `onclick`, not addEventListener: static markup, and this runs on every
  // render — see the fullscreen control's note.
  const cta = doc.getElementById('landing-packs-cta');
  if (cta) {
    cta.textContent = packs ? t(packs.ctaId) : '';
    const enabled = !!packs && packs.ctaEnabled;
    // `aria-disabled` rather than `disabled`, as the inert menu entries take
    // it: a control a screen reader cannot reach is not more honest than one
    // that says it is unavailable.
    cta.setAttribute('aria-disabled', enabled ? 'false' : 'true');
    cta.classList.toggle('off', !enabled);
    cta.onclick = enabled && h.installPack
      ? function () { h.installPack(packs.action, packs.chosen); }
      : null;
  }
  const cancel = doc.getElementById('landing-packs-cancel');
  if (cancel) {
    cancel.textContent = packs ? t(packs.cancelId) : '';
    cancel.onclick = packs && h.pick
      ? function () { h.pick(vm.openEntryId); }
      : null;
  }
}

/**
 * Render one landing view model into `doc`.
 *
 * @param {Document} doc the document holding the `#landing-panel` markup.
 * @param {object} vm the return of `landingViewModel()`.
 * @param {(id: string, params?: object) => string} t string-id resolver.
 * @param {{
 *   pick?: (entryId: string) => void,
 *   confirm?: (action: string) => void,
 *   pickPack?: (file: string) => void,
 *   installPack?: (action: string, file: string) => void,
 *   toggleFullscreen?: () => void,
 *   submitJoin?: (join: object, typed: string) => void,
 * }} [hooks]
 *   `pick` carries an operator's click on a menu entry back to whoever owns
 *   the open-entry memory — which is the caller, not this module (see
 *   `landingViewModel`'s note on `openEntryId`). It is called for EVERY entry
 *   including the inert ones, because whether an entry does anything is the
 *   view model's decision (`nextOpenEntry`) and not a second judgement made
 *   here. `toggleFullscreen` reaches `gui/page-chrome.js`'s one fullscreen
 *   implementation; absent, the control renders and does nothing.
 *
 *   `submitJoin` carries a typed join code back with the open row's own `join`
 *   descriptor (issue #1364) — the descriptor and not the entry id, so the
 *   caller dispatches on the row's `action` and never on which entry it came
 *   from. Nothing is judged here: whether eight letters are a code at all is
 *   `landingJoinAttempt`'s answer, and this module has no more business
 *   parsing one than it has deciding whether an entry may open.
 *
 *   `confirm` is what an operator pressing a confirmation's own control asks
 *   for, carrying the open row's `action` verb (issue #1365). It is a hook and
 *   not something done here for the plainest reason in the module: quitting an
 *   application is not a DOM write, and the two surfaces reach it differently —
 *   the native viewscreen sends a record its host answers with an app-exit, and
 *   the host PAGE supplies no such hook at all, because a browser tab cannot
 *   quit an application. Absent, the control renders and does nothing.
 *
 *   `pickPack` and `installPack` are the mod-pack shelf's two (issue #1366),
 *   and they are two rather than one for the reason the confirmation's press is
 *   not its entry's: highlighting a row is this surface's own memory and costs
 *   nothing, while installing reads an archive off a disk and changes the
 *   catalogue every phone in the room is looking at. `installPack` carries the
 *   open row's `action` verb, exactly as `confirm` does, so this file never
 *   learns what the verb is called. Both absent, the rows render and the button
 *   does nothing — which is what a surface with no shelf behind it would be,
 *   and is also why the view model refuses to open the stage there at all.
 * @param {{dockPanels?: boolean, ownPanelVisibility?: boolean}} [opts]
 *   `dockPanels: false` leaves every borrowed panel where it is, for a surface
 *   that composes the middle column some other way. `server.html` passes
 *   nothing and gets the docking; so does the native viewscreen (#1361),
 *   because the panels it carries are the same borrowed nodes and
 *   `gui/host-landing.css`'s `.landing-docked` blocks are what unwind them
 *   there too — a second arrangement would be a second landing.
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
  const dockPanels = !(opts && opts.dockPanels === false);
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

  // ── The join-code stage (issue #1364) ──────────────────────
  //
  // Two rows open this one panel — Join as Peer and Connect to Host — and every
  // word in it comes off the open row's `join` descriptor, so the panel does
  // not know which route it is serving and would serve a third without an edit.
  //
  // The FIELD'S VALUE IS NEVER WRITTEN HERE, and that is the load-bearing part
  // of this block rather than a shortcut. This function runs again on every
  // menu click and on every refusal, so a render that restored the value would
  // fight an operator who is mid-word, and one that cleared it would delete
  // eight letters the moment their first attempt was refused — at exactly the
  // point they need to see what they typed. The operator owns the field; this
  // owns the sentences around it.
  //
  // `onclick`/`onkeydown` are assigned rather than added, for the same reason
  // the fullscreen control's is: this is STATIC markup and every render would
  // otherwise leave one more listener behind, so the third attempt would join
  // three times.
  const join = vm.join || null;
  setText(doc, 'landing-join-role', join ? t(join.roleId) : '');
  setText(doc, 'landing-join-blurb', join ? t(join.blurbId) : '');
  setText(doc, 'landing-join-label', join ? t(join.labelId) : '');
  setText(doc, 'landing-join-hint', join ? t(join.hintId) : '');
  setText(doc, 'landing-join-submit', join ? t(join.submitId) : '');
  // A refusal, in the words `gui/join-code.js` already gives the phone. Empty
  // when there is none, so a stale sentence cannot outlive the attempt it
  // described.
  setText(doc, 'landing-join-error', join && join.errorId ? t(join.errorId) : '');

  const field = doc.getElementById('landing-join-code');
  const submit = doc.getElementById('landing-join-submit');
  // What both triggers do. Named once so the button and the Return key cannot
  // come to mean two different things — a code typed and submitted with the
  // keyboard is the ordinary case on a surface driven from across a room.
  const sendJoin = function () {
    if (!join || !h.submitJoin) return;
    h.submitJoin(join, field ? String(field.value || '') : '');
  };
  if (field) {
    if (join) {
      field.placeholder = t(join.placeholderId);
      field.setAttribute('aria-label', t(join.labelId));
    }
    // `keydown` and not `keypress`: the latter is deprecated and absent from
    // several remotes' key emulation.
    field.onkeydown = function (event) {
      if (event && event.key === 'Enter') {
        if (typeof event.preventDefault === 'function') event.preventDefault();
        sendJoin();
      }
    };
  }
  if (submit) submit.onclick = sendJoin;

  // ── The confirmation stage (issue #1365) ────────────────────
  //
  // The middle column's next tenant. It is drawn from `vm.confirm` being
  // there at all — never from the stage's name and never from an entry id —
  // so the route that asks before it acts is a row in the entry table and this
  // renderer holds no opinion about which route that is.
  renderConfirm(doc, vm, t, h);

  // ── The mod-pack shelf (issue #1366) ───────────────────────
  //
  // The middle column's last tenant, drawn from `vm.packs` being there at all
  // — never from the stage's name and never from an entry id — so the route
  // that offers a folder is a row in the entry table and this renderer holds no
  // opinion about which route that is.
  renderPacks(doc, vm, t, h);

  // ── The middle column ───────────────────────────────────────────────
  //
  // "Reveals the EXISTING panel" is meant literally: the node named by the
  // open row's `docks` is moved into `#landing-mid` rather than re-rendered
  // here, so a pick still reaches `driveWorldLoad()` down the path it always
  // did and a Start still reaches the resume path down its own, and the
  // mod-pack upload, the docked join panel and the save importer travel with
  // their panel because they are its children.
  //
  // Undocking appends it back to `<body>`, which is a restoration and not an
  // approximation of one: the panel is `position: fixed; inset: 0` with an
  // explicit `z-index: 200`, so where it sits among the body's children
  // decides neither its box nor its paint order. That is what lets this be
  // stateless — no remembered parent to go stale between two documents.
  //
  // What docks is `vm.docks`, a FACT ABOUT THE ROW carried as an element id,
  // and not a comparison of `vm.stage` to a name (issues #1362, #1363). The
  // staged New Game is two stages deep and the picker has to stay put across
  // both — a stage-name test would have undocked it the moment the hulls
  // appeared, taking the World list with it. An ID rather than a boolean is
  // what let Load Game borrow `#save-slots-panel` without this block learning
  // a second name: it reads the row, and the row says which node it wants.
  //
  // The undock sweep runs over the column's OWN children rather than over a
  // list of dockable ids, so this module never has to be told that a new
  // borrower exists. Anything parked here wearing the docked class was parked
  // by this renderer and goes back to `<body>` the moment it is not the open
  // stage's panel.
  if (dockPanels) {
    const mid = doc.getElementById('landing-mid');
    if (mid) {
      const target = vm.docks ? doc.getElementById(vm.docks) : null;
      Array.prototype.slice.call(mid.children || []).forEach(function (child) {
        if (child === target) return;
        if (!child.classList || !child.classList.contains(LANDING_DOCKED_CLASS)) return;
        undock(doc, child);
      });
      if (target) {
        if (target.parentElement !== mid) mid.appendChild(target);
        target.classList.add(LANDING_DOCKED_CLASS);
      }
    }
  }
}

/** Strip the docked class from `el` and hand it back to `doc.body`. */
function undock(doc, el) {
  el.classList.remove(LANDING_DOCKED_CLASS);
  if (doc.body) doc.body.appendChild(el);
}

/**
 * Put every borrowed panel back where it belongs, whatever the landing is
 * doing.
 *
 * The page lifecycle's own escape hatch: `driveWorldLoad()` hides the landing
 * outright the moment a world is chosen, and a borrowed panel must not be left
 * parented inside a hidden one — the second round after a Game Over re-shows
 * `#scenario-panel` by setting its `display`, and a `display` on a node inside
 * a hidden ancestor shows nothing at all.
 *
 * Both borrowers at once, found by the class rather than by id, for the reason
 * the render's sweep is written that way: a slice that teaches a row to borrow
 * a third panel must not also have to remember this function exists.
 */
export function undockLandingPanels(doc) {
  const mid = doc.getElementById('landing-mid');
  if (!mid) return;
  Array.prototype.slice.call(mid.children || []).forEach(function (child) {
    if (child.classList && child.classList.contains(LANDING_DOCKED_CLASS)) undock(doc, child);
  });
}

// Expose for the classic (non-module) script in server.html — the same
// self-registering pattern window.hostScenarioRender uses.
if (typeof window !== 'undefined') {
  window.hostLandingRender = {
    renderHostLanding,
    undockLandingPanels,
    clearLandingEntries,
    LANDING_ENTRY_CLASS,
    LANDING_ENTRY_SELECTOR,
    LANDING_ENTRY_ATTR,
    LANDING_DOCKED_CLASS,
    LANDING_ROOT_CLASSES,
    CONFIRM_DANGER_CLASS,
    LANDING_PACK_SELECTOR,
    LANDING_PACK_ATTR,
    LANDING_NOTE_SELECTOR,
  };
}
