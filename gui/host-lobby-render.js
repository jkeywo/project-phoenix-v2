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

    // Header row: avatar + name + rank
    const header = doc.createElement('div');
    header.className = 'card-header';
    const nameInfo = doc.createElement('div');
    nameInfo.style.cssText = 'display:flex;flex-direction:column;gap:2px;';
    const name = doc.createElement('div');
    name.className = 'card-name';
    name.textContent = c.name;
    const rank = doc.createElement('div');
    rank.className = 'card-rank';
    rank.textContent = c.rank;
    nameInfo.appendChild(name);
    nameInfo.appendChild(rank);
    header.appendChild(nameInfo);

    // Avatar initials
    const avatar = doc.createElement('div');
    avatar.className = 'card-avatar';
    avatar.textContent = c.avatar.text;
    if (c.avatar.placeholder) avatar.style.color = '#556';
    header.appendChild(avatar);
    card.appendChild(header);

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

    // Footer: complexity pill(s)
    if (c.presetPills.length > 0) {
      const footer = doc.createElement('div');
      footer.style.cssText = 'display:flex;gap:6px;align-items:center;margin-top:2px;';
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
  const hintEl = doc.getElementById('lobby-status-hint');
  if (hintEl) {
    hintEl.textContent = t(vm.hint.id, vm.hint.params);
    hintEl.style.color = vm.hint.color;
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

  // ── AI-only launch button ─────────────────────────────────────────
  // Absent from the native lobby document: scenario selection stays on the CLI
  // there, so the document slice strips the one control it would inherit.
  const aiBtn = doc.getElementById('ai-launch-btn');
  if (aiBtn) {
    aiBtn.style.display = vm.aiLaunchVisible ? '' : 'none';
  }
}

// Expose for the classic (non-module) script in server.html — the same
// self-registering pattern window.hostLobbyViewModel uses.
if (typeof window !== 'undefined') {
  window.hostLobbyRender = { renderHostLobby, MONITOR_BUTTON_ATTR };
}
