/**
 * gui/host-lobby-render.js — the host lobby panel's renderer (issue #1325).
 *
 * The DOM half of the pair whose pure half is `gui/host-lobby-view.js`:
 * `hostLobbyViewModel()` decides what the lobby says, this writes it into a
 * document. Both used to live inside `server.html`'s `__updateLobby` — #1229
 * lifted out the decisions, this lifts out the writes — because the native
 * host now renders the SAME lobby on its viewscreen window
 * (`src/native_host/host_lobby/`), and a second implementation of these
 * thirteen element ids is a second lobby that drifts from this one the first
 * time either is touched.
 *
 * ## What it writes, and what it deliberately does not
 *
 * Exactly the contents of `#lobby-panel` — the panel's own visibility, the
 * title/subtitle, the crew counter and its dots, the spectator tag, the ready
 * badge, the countdown, the station-card grid, the aggregate RESERVED chip,
 * the connected-pill list, the status hint, the bridge monitor row (issue
 * #1330) and the AI-launch button.
 *
 * It does NOT touch the viewscreen's other surfaces: the asset-loading
 * overlay, the join panel and the game-over overlay are viewscreen chrome that
 * a lobby push happens to be a convenient moment to reconcile, and they outlive
 * the lobby — the join panel especially, which is toggled back on mid-mission
 * when there is no lobby panel on screen at all.
 *
 * The join panel got the same treatment this module did, in its own module:
 * `gui/host-qr.js` owns its draw and its visibility, both surfaces call it, and
 * both are handed the same `transitions.qrOverlayAction` this module's view
 * model decides (issue #1329). What is left in `server.html`'s glue is what is
 * genuinely that page's: the audio graph, the fleet freeze, the mesh pump, and
 * what a click on the QR does in a desktop browser.
 *
 * Every write is guarded on the element existing, which is what lets one
 * renderer serve two documents: the native lobby document carries the panel
 * markup and no `#ai-launch-btn` (this slice's native lobby is read-only —
 * selection stays on the CLI), and the button's branch simply does nothing.
 *
 * ## `t` is passed in, not imported
 *
 * The view model returns `{ id, params }` pairs for text whose string id
 * depends on the data (`gui/lobby-view.js`'s `statusLine` convention). The
 * caller resolves them, because the two consumers reach their string table
 * differently: `server.html` holds a classic-script `t()` closed over
 * `window.phStrings`, and the native lobby document imports `gui/strings.js`
 * directly. Neither is this module's business.
 */

/**
 * The attribute a monitor button carries its display's stable identity in
 * (issue #1330).
 *
 * Exported because the press half lives elsewhere:
 * `src/native_host/host_lobby/host_lobby_link.js` delegates a click listener
 * off the row's container and reads this attribute to know which display was
 * pressed. A literal spelled in both files is a button that silently does
 * nothing the first time either is touched.
 *
 * A `data-` attribute rather than the button's text, deliberately: the text is
 * localised and elided, and the identity is a machine key that must reach the
 * host byte-for-byte or the layout law refuses it as a monitor this bridge does
 * not have.
 */
export const MONITOR_BUTTON_ATTR = 'data-monitor';
export const GM_MONITOR_BUTTON_ATTR = 'data-gm-monitor';

/**
 * The attributes a station's screen button carries (issue #1331).
 *
 * Two of them, and deliberately NOT `data-monitor`: the viewscreen row's
 * delegated listener matches on that attribute, and a station button carrying
 * it would move the shared view instead of opening a console. So a screen
 * button is identified by `data-station` (which station's console) and
 * `data-screen` (which display, or empty for the off state), and the listener
 * in `src/native_host/host_lobby/host_lobby_link.js` tests for the station
 * attribute first.
 *
 * The off button carries `data-screen=""` rather than omitting the attribute:
 * an absent attribute and an empty one are the same to `closest()` but not to
 * `getAttribute`, and "off" has to be a value the listener can act on rather
 * than a hole it has to guess the meaning of.
 */
export const STATION_BUTTON_ATTR = 'data-station';
export const STATION_SCREEN_ATTR = 'data-screen';

/**
 * Render one lobby view model into `doc`.
 *
 * @param {Document} doc the document holding the `#lobby-panel` markup.
 * @param {object} vm the return of `hostLobbyViewModel()`.
 * @param {(id: string, params?: object) => string} t string-id resolver.
 * @param {{revealChrome?: boolean}} [opts] `revealChrome` forces the panel
 *   visible even when the phase says otherwise. It is the native host's
 *   permanent-surface reveal (issue #1325): on the viewscreen the lobby
 *   chrome yields at mission start and one host key brings it back, and that
 *   decision is a native one (`native_host::host_lobby::reveal`) rather than
 *   anything the payload can carry. `server.html` passes nothing and gets the
 *   phase-only behaviour it always had.
 */
export function renderHostLobby(doc, vm, t, opts) {
  const revealChrome = !!(opts && opts.revealChrome);

  // ── Show/hide the panel ─────────────────────────────────────────────
  // NOTE (server.html): do NOT hide the Bevy canvas (#canvas) alongside this.
  // The lobby panel already covers it completely (z-index:180, solid
  // background). Hiding the canvas with display:none causes Bevy to see a 0×0
  // window and throttle / suspend its rAF loop, which delays SimState delivery
  // after game start and breaks the smoke tests. The canvas must stay in the
  // render tree at all times.
  const panel = doc.getElementById('lobby-panel');
  if (panel) panel.style.display = (vm.transitions.showPanel || revealChrome) ? '' : 'none';

  // ── Title / subtitle ──────────────────────────────────────────────
  // Both are the world's authored `[global] title` / `description`, which
  // combat_test.toml holds as string ids. They arrive resolved: every host
  // channel crosses localiseHostPayload before it reaches the view model
  // (issue #949).
  const titleEl = doc.getElementById('lobby-title');
  const subEl = doc.getElementById('lobby-subtitle');
  if (titleEl) titleEl.textContent = vm.title || t('server.unknown_scenario');
  if (subEl) subEl.textContent = vm.subtitle;

  // ── Crew count ────────────────────────────────────────────────────
  const crewEl = doc.getElementById('lobby-crew-count');
  const dotsEl = doc.getElementById('lobby-crew-dots');
  if (crewEl) crewEl.textContent = vm.crew.count + '/' + vm.crew.max;

  // Crew dot indicators
  if (dotsEl) {
    dotsEl.innerHTML = '';
    for (const filled of vm.crew.dots) {
      const dot = doc.createElement('div');
      dot.className = 'crew-dot' + (filled ? ' filled' : '');
      dotsEl.appendChild(dot);
    }
  }

  // Spectator tag
  const specTag = doc.getElementById('lobby-spectator-tag');
  if (specTag) {
    specTag.style.display = vm.crew.spectatorTag.visible ? 'inline' : 'none';
    if (vm.crew.spectatorTag.visible) specTag.textContent = '+' + vm.crew.spectatorTag.count;
  }

  // ── Ready badge ───────────────────────────────────────────────────
  const badge = doc.getElementById('lobby-ready-badge');
  if (badge) {
    badge.textContent = t(vm.readyBadge.id, vm.readyBadge.params);
    badge.className = vm.readyBadge.className;
  }

  // ── Countdown display ─────────────────────────────────────────────
  const cdEl = doc.getElementById('lobby-countdown');
  if (cdEl) {
    if (vm.countdown.visible) {
      cdEl.textContent = String(vm.countdown.secs);
      cdEl.style.display = 'flex';
    } else {
      cdEl.style.display = 'none';
    }
  }

  // ── Station grid ──────────────────────────────────────────────────
  // The early return is the original's: with no grid there is no lobby body to
  // fill, and everything below it lives in that body. Kept rather than
  // flattened, so a document that carries the header alone renders the header
  // alone instead of a half-populated rail.
  const grid = doc.getElementById('station-grid');
  if (!grid) return;
  grid.innerHTML = '';

  for (const c of vm.cards) {
    const card = doc.createElement('div');
    card.className = 'station-card' + (c.claimed ? ' claimed' : '');

    // Header row: avatar + name + rank. The design (issue #1358) puts the
    // avatar FIRST and the identity beside it, so the DOM order follows —
    // a reader who hears the row gets it in the order a viewer sees it.
    const header = doc.createElement('div');
    header.className = 'card-header';

    // Avatar initials. The placeholder is a CLASS rather than the inline
    // colour this used to write: the one thing that tells an unclaimed seat
    // from a claimed one across a room now follows the palette in
    // gui/host-lobby.css instead of a hex frozen into this module.
    const avatar = doc.createElement('div');
    avatar.className = 'card-avatar' + (c.avatar.placeholder ? ' placeholder' : '');
    avatar.textContent = c.avatar.text;
    header.appendChild(avatar);

    const nameInfo = doc.createElement('div');
    nameInfo.className = 'card-id';
    const name = doc.createElement('div');
    name.className = 'card-name';
    name.textContent = c.name;
    const rank = doc.createElement('div');
    rank.className = 'card-rank';
    rank.textContent = c.rank;
    nameInfo.appendChild(name);
    nameInfo.appendChild(rank);
    header.appendChild(nameInfo);
    card.appendChild(header);

    // Who holds the seat, in words (issue #1358). The avatar carries two
    // letters of it, which is an identifier rather than a name — and a room
    // deciding whether to wait for somebody needs the name. A free Station
    // says what will fly it instead: the Backfill rating, which is what
    // actually runs its systems when nobody sits down.
    const holder = doc.createElement('div');
    holder.className = 'card-holder' + (c.holder.text ? '' : ' none');
    holder.textContent = c.holder.text || t(c.holder.id, c.holder.params);
    card.appendChild(holder);

    // Console chips
    const chips = doc.createElement('div');
    chips.className = 'card-consoles';
    for (const chipLabel of c.consoles) {
      const chip = doc.createElement('span');
      chip.className = 'console-chip';
      chip.textContent = chipLabel;
      chips.appendChild(chip);
    }
    card.appendChild(chips);

    // ── This station's screen row (issue #1331) ──────────────────────────
    // One button per display this console may open on, an off button that
    // closes it, and — on a bridge with nowhere but the viewscreen — a line
    // saying so instead. Absent entirely on the browser host, whose view model
    // carries no bridge at all.
    if (c.screens) renderStationScreens(doc, card, c.screens, t);

    // Footer: complexity pill(s). A class rather than the inline flex this
    // used to carry — the console chips above it are ruled the same way, and
    // the design draws the two as one meta row.
    if (c.presetPills.length > 0) {
      const footer = doc.createElement('div');
      footer.className = 'card-foot';
      for (const pill of c.presetPills) {
        const pillEl = doc.createElement('span');
        pillEl.className = 'complexity-pill' + (pill.low ? ' low' : '');
        pillEl.textContent = t(pill.id);
        footer.appendChild(pillEl);
      }
      card.appendChild(footer);
    }

    grid.appendChild(card);
  }

  // Aggregate RESERVED chip (visible only in compact mode via CSS)
  const aggregate = doc.getElementById('reserved-aggregate');
  if (aggregate) {
    if (vm.reservedChip.active) {
      aggregate.classList.add('active');
      // Two ids, not one with a JS-side `s`: a language whose plural rule is
      // not English's cannot be served by suffixing a letter.
      aggregate.textContent = t(vm.reservedChip.id, vm.reservedChip.params);
    } else {
      aggregate.classList.remove('active');
      aggregate.textContent = '';
    }
  }

  // ── Spectator list ─────────────────────────────────────────────────
  const specList = doc.getElementById('lobby-spectator-list');
  if (specList) {
    specList.innerHTML = '';
    specList.style.color = '';
    for (const p of vm.spectatorPills) {
      const pill = doc.createElement('span');
      if (p.kind === 'crew') {
        pill.className = 'spectator-pill';
        pill.textContent = p.text;
      } else if (p.kind === 'waiting') {
        pill.className = 'spectator-pill waiting';
        pill.textContent = t(p.id, p.params);
      } else {
        pill.className = 'spectator-empty';
        pill.textContent = t(p.id);
      }
      specList.appendChild(pill);
    }
  }

  // ── Status hint ───────────────────────────────────────────────────
  // The tone is a CLASS, not an inline colour (issue #1358). The view model
  // used to carry a hex, which put three values of the palette in a pure
  // module that cannot see a stylesheet and could not follow a retint; what it
  // decides is whether the line is LIVE, and gui/host-lobby.css decides what
  // live looks like.
  const hintEl = doc.getElementById('lobby-status-hint');
  if (hintEl) {
    hintEl.textContent = t(vm.hint.id, vm.hint.params);
    hintEl.className = 'lobby-status-hint' + (vm.hint.tone ? ' ' + vm.hint.tone : '');
  }

  // ── Bridge monitor row (issue #1330) ──────────────────────────────
  // One button per connected monitor, the viewscreen's marked. Present in
  // BOTH documents' markup and filled in neither unless a bridge reported a
  // roster: `vm.monitorRow` is null on the host page, which has no monitors,
  // so the row is hidden by the same branch that hides it on a native host
  // whose winit has not enumerated its displays yet.
  const row = doc.getElementById('monitor-row');
  if (row) {
    const model = vm.monitorRow;
    row.style.display = model ? '' : 'none';

    const buttons = doc.getElementById('monitor-row-buttons');
    if (buttons) {
      // Replaced wholesale, like the station grid: the press listener is
      // delegated off this container precisely because these do not survive.
      buttons.innerHTML = '';
      for (const b of (model ? model.buttons : [])) {
        const el = doc.createElement('button');
        el.type = 'button';
        el.className = 'monitor-button' + (b.viewscreen ? ' viewscreen' : '');
        el.setAttribute(MONITOR_BUTTON_ATTR, b.identity);
        // `aria-pressed` rather than a colour alone: which monitor is showing
        // the viewscreen is the one fact this row carries, and it must not be
        // legible only to somebody who can tell two blues apart (WCAG 1.4.1).
        // The mark below says the same thing in words for the same reason.
        el.setAttribute('aria-pressed', b.viewscreen ? 'true' : 'false');
        el.disabled = !!b.disabled;

        const name = doc.createElement('span');
        name.className = 'monitor-button-name';
        name.textContent = t(b.label.id, b.label.params);
        el.appendChild(name);

        for (const mark of b.marks) {
          const markEl = doc.createElement('span');
          markEl.className = 'monitor-button-mark';
          markEl.textContent = t(mark.id, mark.params);
          el.appendChild(markEl);
        }

        // What this display is already holding. A console never moves aside
        // for the viewscreen (the layout law refuses the press rather than
        // evicting it), so a button that did not say so offered a press that
        // could only ever come back as a refusal.
        if (b.occupants) {
          const held = doc.createElement('span');
          held.className = 'monitor-button-stations';
          held.textContent = t(b.occupants.id, b.occupants.params);
          el.appendChild(held);
        }
        buttons.appendChild(el);
      }
    }

    // Whatever the layout law said about the last press or the last cable
    // that moved — a refusal the operator must see, or a monitor that went
    // away and took the viewscreen's chosen home with it.
    const notices = doc.getElementById('monitor-row-notice');
    if (notices) {
      notices.innerHTML = '';
      for (const n of (model ? model.notices : [])) {
        const line = doc.createElement('div');
        line.className = 'monitor-row-notice-line';
        line.textContent = t(n.id, n.params);
        notices.appendChild(line);
      }
    }
  }

  let gmRow = doc.getElementById('gm-monitor-row');
  if (!gmRow && vm.gmRow && row) {
    gmRow = doc.createElement('div');
    gmRow.id = 'gm-monitor-row';
    gmRow.className = 'monitor-row';
    row.after(gmRow);
  }
  if (gmRow) {
    gmRow.hidden = !vm.gmRow;
    gmRow.innerHTML = '';
    if (vm.gmRow) {
      const label = doc.createElement('span');
      label.id = 'gm-monitor-row-label';
      label.textContent = t('server.gm_monitor_row.label');
      gmRow.appendChild(label);
      const buttons = doc.createElement('div');
      buttons.className = 'monitor-row-buttons';
      buttons.setAttribute('role', 'group');
      buttons.setAttribute('aria-labelledby', label.id);
      for (const b of [{ identity: '', label: { id: 'server.station_row.off', params: {} }, ...vm.gmRow.off }, ...vm.gmRow.buttons]) {
        const button = doc.createElement('button');
        button.type = 'button';
        button.className = 'monitor-button' + (b.selected ? ' viewscreen' : '');
        button.setAttribute(GM_MONITOR_BUTTON_ATTR, b.identity);
        button.setAttribute('aria-pressed', b.selected ? 'true' : 'false');
        button.disabled = !!b.disabled;
        button.textContent = t(b.label.id, b.label.params);
        if (b.reason) {
          const reason = doc.createElement('span');
          reason.className = 'monitor-button-stations';
          reason.textContent = t(b.reason.id, b.reason.params);
          button.appendChild(reason);
        }
        buttons.appendChild(button);
      }
      gmRow.appendChild(buttons);
      const message = doc.createElement('span');
      message.className = 'monitor-row-notice';
      message.textContent = t(vm.gmRow.message.id, vm.gmRow.message.params);
      gmRow.appendChild(message);
    }
  }

  // ── AI-only launch button ─────────────────────────────────────────
  // Absent from the native lobby document: scenario selection stays on the CLI
  // there, so the document slice strips the one control it would inherit.
  const aiBtn = doc.getElementById('ai-launch-btn');
  if (aiBtn) {
    aiBtn.style.display = vm.aiLaunchVisible ? '' : 'none';
  }
}

/**
 * Draw one station card's screen row into `card` (issue #1331).
 *
 * Split out of the card loop above because it is a control strip rather than
 * card content: every element it makes is operable, and the rules that go with
 * that — a real `<button>` so a keyboard reaches it, `aria-pressed` so the
 * chosen screen is legible without colour, `disabled` so a full screen is
 * skipped rather than offered and refused — are all in one place instead of
 * threaded through the card's presentation.
 *
 * @param {Document} doc
 * @param {Element} card the `.station-card` this row belongs to.
 * @param {object} row one entry of `hostLobbyStationRows()`.
 * @param {(id: string, params?: object) => string} t
 */
function renderStationScreens(doc, card, row, t) {
  const strip = doc.createElement('div');
  strip.className = 'station-screens';

  const label = doc.createElement('span');
  label.className = 'station-screens-label';
  label.textContent = t('server.station_row.label');
  strip.appendChild(label);

  // A bridge with one display offers this station nothing, so it says why
  // rather than showing an empty strip the operator would read as broken.
  if (row.message) {
    const note = doc.createElement('span');
    note.className = 'station-screens-message';
    note.textContent = t(row.message.id, row.message.params);
    strip.appendChild(note);
    card.appendChild(strip);
    return;
  }

  const button = (screen, selected, disabled, text, reason) => {
    const el = doc.createElement('button');
    el.type = 'button';
    el.className = 'station-screen-button' + (selected ? ' selected' : '');
    el.setAttribute(STATION_BUTTON_ATTR, row.station);
    el.setAttribute(STATION_SCREEN_ATTR, screen);
    // Which screen this console is on is the one fact the row carries, and it
    // must not be legible only to somebody who can tell two blues apart
    // (WCAG 1.4.1) — the same reason the monitor row marks its viewscreen with
    // `aria-pressed` as well as a border.
    el.setAttribute('aria-pressed', selected ? 'true' : 'false');
    if (disabled) el.disabled = true;
    const name = doc.createElement('span');
    name.className = 'station-screen-name';
    name.textContent = text;
    el.appendChild(name);
    if (reason) {
      const why = doc.createElement('span');
      why.className = 'station-screen-reason';
      why.textContent = t(reason.id, reason.params);
      el.appendChild(why);
    }
    return el;
  };

  // Off first, so the row reads "closed, or one of these" left to right and the
  // control that always works is the one under the operator's thumb.
  strip.appendChild(
    button('', row.off.selected, false, t('server.station_row.off'), null),
  );
  for (const b of row.buttons) {
    strip.appendChild(
      button(b.identity, b.selected, b.disabled, t(b.label.id, b.label.params), b.reason),
    );
  }
  card.appendChild(strip);
}

// Expose for the classic (non-module) script in server.html — the same
// self-registering pattern window.hostLobbyViewModel uses.
if (typeof window !== 'undefined') {
  window.hostLobbyRender = {
    renderHostLobby,
    MONITOR_BUTTON_ATTR,
    GM_MONITOR_BUTTON_ATTR,
    STATION_BUTTON_ATTR,
    STATION_SCREEN_ATTR,
  };
}
