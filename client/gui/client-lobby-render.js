/**
 * gui/client-lobby-render.js — the client lobby's renderer (issue #1369).
 *
 * The DOM half of the pair whose pure halves are `gui/lobby-view.js`
 * (`lobbyViewModel`) and `gui/station-roster.js` (`buildStationRoster`). Those
 * two decided what the lobby says; roughly 325 lines of `createElement` sat on
 * top of them inside `client.html`'s `renderLobby`, deciding a few more things
 * on the way past. This is that code, lifted out whole.
 *
 * The shape is the one `gui/spectator-view.js` already proved on this side of
 * the wire — a pure model beside a `renderSpectatorDom` writer, which is why
 * `renderSpectator()` in the page is thirty-five lines — and the conventions
 * are the host's, from `gui/host-lobby-render.js`:
 *
 *   - **document first.** Every read is `doc.getElementById`, never a global
 *     `document`, so a test renders into its own document and two surfaces can
 *     share one renderer if a second one ever wants this lobby.
 *   - **every write guarded.** A missing element is skipped, not thrown over.
 *     The lobby is a boot race — `client.html` loads its modules as ES modules
 *     in `<head>` and paints from the first server message — and a partially
 *     mounted shell must paint what it has rather than stopping at the first
 *     hole.
 *   - **`t` is passed in, not imported.** The page holds a classic-script `t()`
 *     closed over `window.phStrings`; a test imports `gui/strings.js` directly.
 *     Neither is this module's business.
 *   - **actions arrive as handlers.** See below.
 *
 * ## Why the actions are injected
 *
 * Six controls in the old glue closed over page-local mutable state:
 * `releaseStation` (which writes `releaseArmed` and calls `scheduleRender`),
 * the claim button (which writes `pendingMidGameClaim` and reads
 * `uiState.phase`), the console chip (which writes `lobbyConsole`), the
 * complexity button, and the ready and spectate buttons (which `send()` on the
 * live transport). A closure over page state is exactly what cannot move into
 * a module, so it does not: the page keeps its own state and hands this
 * renderer six functions.
 *
 * Each control also carries a `data-` attribute naming what it acts on —
 * `data-station`, `data-console`, `data-rating` — for the same reason the host
 * renderer exports `MONITOR_BUTTON_ATTR` and friends: a caller that would
 * rather delegate one listener off the container than take a callback per
 * control can match on the attribute, and a literal spelled in two files is a
 * button that silently does nothing the first time either is touched.
 *
 * ## Listeners: `onclick` where the element survives, `addEventListener` where
 * it does not
 *
 * The roster, the detail panel and the GM list are rebuilt wholesale on every
 * paint, so a listener attached to a fresh node dies with it. `#ready-btn` and
 * `#spectate-btn` are in the page's static markup and survive every paint, so
 * they take `onclick =` — assigning REPLACES, where `addEventListener` would
 * stack one more copy of the handler per render until a single tap sent a
 * dozen `SetReady`s. That asymmetry is deliberate and is what the old inline
 * code did too.
 */

/**
 * The attributes a lobby control carries its subject in.
 *
 * `data-station` is on the ROW (matching what the smoke specs already select
 * on) as well as on nothing else, so `closest('[data-station]')` from a claim
 * press reaches the seat; `data-console` and `data-rating` are on their own
 * controls inside the detail panel.
 */
export const STATION_ROW_ATTR = 'data-station';
export const CONSOLE_CHIP_ATTR = 'data-console';
export const RATING_BUTTON_ATTR = 'data-rating';

/** Resolve a `{ id, params }` / `{ text }` label pair. Neither is a decision. */
function label(pair, t) {
  if (!pair) return '';
  if (typeof pair.text === 'string') return pair.text;
  return pair.id ? t(pair.id, pair.params || {}) : '';
}

/**
 * Render one lobby view model into `doc`.
 *
 * @param {Document} doc the document holding `client.html`'s lobby markup.
 * @param {object} vm the return of `lobbyViewModel()`.
 * @param {(id: string, params?: object) => string} t string-id resolver.
 * @param {{ release?: () => void,
 *           claim?: (row: object) => void,
 *           selectConsole?: (consoleId: string) => void,
 *           selectRating?: (rating: string) => void,
 *           setReady?: (ready: boolean) => void,
 *           setSpectator?: (spectator: boolean) => void }} [handlers]
 *        The six actions the old inline closures performed. Each is optional:
 *        a surface that renders the lobby read-only passes none and gets a
 *        lobby whose controls are inert rather than one that throws.
 */
export function renderClientLobby(doc, vm, t, handlers) {
  const on = handlers || {};
  renderGmPresence(doc, vm, t);
  renderRoster(doc, vm, t, on);
  renderDetail(doc, vm, t, on);
  renderHeader(doc, vm, t);
  renderReadyButton(doc, vm, t, on);
  renderSpectateButton(doc, vm, t, on);

  const statusLine = doc.getElementById('status-line');
  if (statusLine && vm.statusLine) {
    statusLine.textContent = t(vm.statusLine.id, vm.statusLine.params);
  }
}

/**
 * Equal GM peers, crew-public but never a Station or a spectator row — so its
 * own labelled region rather than a line in the roster.
 */
function renderGmPresence(doc, vm, t) {
  const region = doc.getElementById('gm-presence');
  const list = doc.getElementById('gm-presence-list');
  if (!region || !list || !vm.gmGroup) return;
  region.setAttribute('aria-hidden', vm.gmGroup.visible ? 'false' : 'true');
  const heading = doc.getElementById('gm-presence-heading');
  if (heading) heading.textContent = t(vm.gmGroup.headingId);
  list.innerHTML = '';
  for (const gm of vm.gmGroup.entries) {
    const row = doc.createElement('span');
    row.className = 'gm-presence-pill' + (gm.connected ? '' : ' disconnected');
    row.setAttribute('role', 'listitem');
    row.dataset.gmId = gm.id;
    row.dataset.ready = gm.ready ? 'true' : 'false';
    row.textContent = t(gm.labelId, { name: gm.name || gm.id })
      + ' · ' + t(gm.readinessLabelId);
    list.appendChild(row);
  }
}

/** The station roster: one row per claimable seat, with its action control. */
function renderRoster(doc, vm, t, on) {
  const list = doc.getElementById('station-list');
  if (!list) return;
  // Station help lives in Settings; keep the lobby roster visible.
  list.style.display = '';
  list.innerHTML = '';

  for (const row of vm.rows) {
    const rowEl = doc.createElement('div');
    rowEl.className = row.rowClass;
    if (row.id) rowEl.setAttribute(STATION_ROW_ATTR, row.id);

    const glyph = doc.createElement('div');
    glyph.className = 'glyph';
    glyph.textContent = row.glyph;
    rowEl.appendChild(glyph);

    const info = doc.createElement('div');
    info.className = 'info';
    const nameEl = doc.createElement('div');
    nameEl.className = 'name';
    nameEl.textContent = row.label;
    info.appendChild(nameEl);

    const meta = doc.createElement('div');
    meta.className = 'meta';
    if (row.rank) {
      const rankEl = doc.createElement('span');
      // Classed, not bare: issue #1370 gives the rank its own type in the
      // roster's meta line, and a rule cannot select an unclassed span
      // without also catching whatever field lands beside it next.
      rankEl.className = 'rank';
      rankEl.textContent = row.rank;
      meta.appendChild(rankEl);
    }
    if (row.chipId) {
      const chip = doc.createElement('span');
      chip.className = 'cons';
      chip.textContent = row.chipLabel;
      meta.appendChild(chip);
    }
    info.appendChild(meta);

    // The job, before the claim (PRD #1023 module 4). Rendered for every row
    // kind — free, taken and mine — so the roster reads as a crew manifest
    // rather than a list of buttons.
    if (row.description) {
      const desc = doc.createElement('div');
      desc.className = 'desc';
      desc.textContent = row.description;
      info.appendChild(desc);
    }
    if (row.occupant) {
      const occ = doc.createElement('div');
      occ.className = 'occupant';
      occ.textContent = row.occupant;
      info.appendChild(occ);
    }

    // A free seat this player is INELIGIBLE for (issue #1103 AC1): explained
    // privately, in functional terms, right on the row. The reason is
    // local-only — it is never sent to the host or to another player.
    if (row.ineligibleReasonId) {
      const why = doc.createElement('div');
      why.className = 'ineligible-reason';
      const fns = (row.ineligibleFunctionIds || []).map(id => t(id)).join(', ');
      why.textContent = t(row.ineligibleReasonId, { functions: fns });
      info.appendChild(why);
    }
    rowEl.appendChild(info);

    const action = doc.createElement('div');
    action.className = 'action';
    const btn = doc.createElement('button');
    btn.className = row.actionClass;
    btn.textContent = label(row.actionLabel, t);
    if (row.actionDisabled) {
      btn.disabled = true;
    } else if (row.button === 'release') {
      btn.addEventListener('click', () => { if (on.release) on.release(); });
    } else {
      btn.addEventListener('click', () => { if (on.claim) on.claim(row); });
    }
    action.appendChild(btn);
    rowEl.appendChild(action);
    list.appendChild(rowEl);
  }
}

/** The held-seat detail panel: name, description, console chip, complexity. */
function renderDetail(doc, vm, t, on) {
  const detail = doc.getElementById('detail-panel');
  if (!detail || !vm.detail) return;
  detail.className = vm.detail.active ? 'active' : 'idle';

  const header = detail.querySelector('.detail-header');
  if (header) {
    header.innerHTML = '';
    const nameSpan = doc.createElement('span');
    nameSpan.className = 'detail-station-name';
    nameSpan.textContent = label(vm.detail.title, t);
    header.appendChild(nameSpan);
    // The LEAVE control is the row's RELEASE under another resting label, and
    // it shares the one handler: both are the same arm→confirm (#771 AC3/AC4).
    if (vm.detail.active && vm.detail.releaseLabel) {
      const releaseBtn = doc.createElement('button');
      releaseBtn.className = 'detail-release-btn';
      releaseBtn.textContent = label(vm.detail.releaseLabel, t);
      releaseBtn.addEventListener('click', () => { if (on.release) on.release(); });
      header.appendChild(releaseBtn);
    }
  }

  // The same authored line the row carried, restated where the player now
  // sits (PRD #1023 module 4).
  const descEl = detail.querySelector('.detail-station-desc');
  if (descEl) descEl.textContent = vm.detail.stationDescription || '';

  const consolesArea = detail.querySelector('.detail-consoles');
  if (consolesArea) {
    consolesArea.innerHTML = '';
    for (const c of vm.detail.consoles) {
      const chip = doc.createElement('span');
      chip.className = 'chip' + (c.selected ? ' selected' : '');
      chip.setAttribute(CONSOLE_CHIP_ATTR, c.id);
      chip.textContent = c.label;
      chip.addEventListener('click', () => { if (on.selectConsole) on.selectConsole(c.id); });
      consolesArea.appendChild(chip);
    }
  }

  // Complexity toggle: purely data-driven off the station's declared ratings
  // (the view model omits it when the station only has its base rating).
  const ratingsArea = detail.querySelector('.detail-ratings');
  if (ratingsArea) {
    ratingsArea.innerHTML = '';
    for (const r of (vm.detail.ratings ? vm.detail.ratings.list : [])) {
      const ratingBtn = doc.createElement('button');
      ratingBtn.className = 'rating-btn' + (r.active ? ' active' : '');
      ratingBtn.setAttribute(RATING_BUTTON_ATTR, r.name);
      ratingBtn.textContent = r.label;
      // The active rating is already in force, so pressing it sends nothing.
      if (!r.active) {
        ratingBtn.addEventListener('click', () => {
          if (on.selectRating) on.selectRating(r.name);
        });
      }
      ratingsArea.appendChild(ratingBtn);
    }
  }
}

/**
 * The lobby header: the ship identity row and the crew/readiness readout.
 *
 * Restyled to the "Client lobby — portrait frame" artboard in issue #1369, and
 * the styling is driven from HERE rather than from a class the page sets
 * elsewhere: `vm.readyPill.className` is this renderer's output and
 * `#ready-pill.go` in `client.html` is the only rule that reads it, so the
 * badge's appearance is a function of the view model like everything else on
 * the surface.
 *
 * That class is a colour swap, and a colour alone is not a cue (WCAG 1.4.1).
 * The non-colour cue is the pill's own TEXT — `client.all_crew_ready` against
 * `client.awaiting_crew`, picked by the same view model — because text is what
 * a screen reader announces. A `data-` attribute would NOT have served either
 * end of that: assistive technology cannot see one, and no rule in
 * `client.html` selects on one here — which is why this header stamps none.
 */
function renderHeader(doc, vm, t) {
  const crewEl = doc.getElementById('crew-display');
  if (crewEl && vm.crew) {
    crewEl.textContent = vm.crew.filled + '/' + vm.crew.max;
  }

  const pill = doc.getElementById('ready-pill');
  if (pill && vm.readyPill) {
    pill.textContent = label(vm.readyPill.label, t);
    pill.className = vm.readyPill.className;
  }
}

/** The per-player ready button (which replaced the captain-only Engage). */
function renderReadyButton(doc, vm, t, on) {
  const readyBtn = doc.getElementById('ready-btn');
  if (!readyBtn || !vm.readyBtn) return;
  if (!vm.readyBtn.visible) {
    readyBtn.style.display = 'none';
    return;
  }
  readyBtn.style.display = 'block';
  readyBtn.disabled = false;
  readyBtn.textContent = label(vm.readyBtn.label, t);
  readyBtn.className = vm.readyBtn.className;
  // Assigned, not added: this element survives every paint (see the module
  // note), so a second listener would be a second SetReady per tap.
  readyBtn.onclick = () => { if (on.setReady) on.setReady(vm.readyBtn.sendReady); };
}

/**
 * The Spectate/Join toggle (issue #1105): opt into or out of the explicit
 * Spectator role. The view model decides which way the press goes.
 */
function renderSpectateButton(doc, vm, t, on) {
  const btn = doc.getElementById('spectate-btn');
  if (!btn) return;
  if (!vm.spectateBtn || !vm.spectateBtn.visible) {
    btn.style.display = 'none';
    return;
  }
  btn.style.display = 'block';
  btn.textContent = label(vm.spectateBtn.label, t);
  btn.onclick = () => { if (on.setSpectator) on.setSpectator(vm.spectateBtn.sendSpectator); };
}

/**
 * The "Mods active" lobby list (issue #990).
 *
 * Its own entry point rather than a branch of `renderClientLobby`, because it
 * is deliberately outside the roster's boot-race guard: a mid-round spectator
 * who never sees a station list must still see what packs the session is
 * running. The rows come from `gui/active-packs-view.js`; the caller folds
 * them, this writes them.
 *
 * @param {Document} doc
 * @param {Array<{ name: string, version?: string }>} rows
 * @param {(id: string, params?: object) => string} t
 */
export function renderClientMods(doc, rows, t) {
  const el = doc.getElementById('lobby-mods');
  if (!el) return;
  const list = rows || [];
  el.innerHTML = '';
  if (list.length === 0) {
    // Nothing renders (and the row is hidden) when no packs are applied, so
    // the base game shows no empty banner.
    el.setAttribute('aria-hidden', 'true');
    return;
  }
  el.setAttribute('aria-hidden', 'false');
  const heading = doc.createElement('div');
  heading.className = 'mods-active-heading';
  heading.textContent = t('client.mods_active');
  el.appendChild(heading);
  for (const row of list) {
    const rowEl = doc.createElement('div');
    rowEl.className = 'mods-active-row';
    const nameEl = doc.createElement('span');
    nameEl.className = 'mods-active-name';
    nameEl.textContent = row.name;
    rowEl.appendChild(nameEl);
    if (row.version) {
      const verEl = doc.createElement('span');
      verEl.className = 'mods-active-version';
      verEl.textContent = t('client.mods_active_version', { version: row.version });
      rowEl.appendChild(verEl);
    }
    el.appendChild(rowEl);
  }
}

// Expose for the non-module inline script in client.html — the same
// self-registering pattern window.lobbyViewModel uses.
if (typeof window !== 'undefined') {
  window.clientLobbyRender = {
    renderClientLobby,
    renderClientMods,
    STATION_ROW_ATTR,
    CONSOLE_CHIP_ATTR,
    RATING_BUTTON_ATTR,
  };
}
