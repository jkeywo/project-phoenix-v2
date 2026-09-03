/**
 * gui/host-qr.js — the join panel, for every surface that shows one
 * (issue #1329).
 *
 * The QR a crew scans is drawn in exactly one place, and this is it. Before
 * this module the draw lived inside `server.html`'s `paintJoinPanel`, which was
 * fine while a browser tab was the only host — and stopped being fine when the
 * native host started compositing the same lobby onto its viewscreen
 * (`src/native_host/host_lobby/`), because the alternative to sharing was a
 * second `QRCode.toCanvas` call against the same element ids, drifting from
 * this one the first time either was touched. The same argument
 * `gui/host-lobby-render.js` makes for the lobby panel, applied to the panel
 * that floats over it.
 *
 * ## What it owns
 *
 * The `#qr-panel` nest (`#qr`, `#qr-link`, `#qr-url`, `#join-code`) and the
 * visibility of the `#overlay` that carries it. Both surfaces call the same
 * five functions; neither writes those elements itself.
 *
 * ## The visibility law, and where it comes from
 *
 * Unchanged from what `server.html` has always done, now stated once:
 *
 *   | when | the join panel |
 *   |---|---|
 *   | the Lobby phase | shown — this is what it is for |
 *   | Loading, GameOver | hidden |
 *   | InProgress | left exactly as it was |
 *
 * The phase half arrives as `hostLobbyViewModel().transitions.qrOverlayAction`
 * (`'show'` / `'hide'` / `null`) — the view model already decided it, for both
 * surfaces, and [`applyQrPhase`] only carries it out. `null` is the
 * load-bearing case: **during a mission nothing phase-driven touches the
 * panel**, so a late arrival's code can be toggled back on and stay on.
 *
 * The toggle half is [`toggleQr`], reached from three places — the host page's
 * settings cog (`gui/server-settings.js` → `__hostToggleQrCode`), a phone's
 * `ToggleQrCode`, and the native surface's own control — all of which land on
 * this one function.
 *
 * ## The state is the DOM
 *
 * There is no `qrVisible` variable here, and `server.html`'s was deleted. Two
 * surfaces and four callers cannot share a module-level flag (the native lobby
 * document and the host page are different documents in different engines), and
 * a flag that can disagree with `#overlay.style.display` is a bug waiting for
 * the one path that forgets to update it — which is precisely how the host page
 * used to get a QR that the cog thought was hidden. `doc` is passed in for the
 * same reason `renderHostLobby` takes one.
 *
 * ## `t` is not imported
 *
 * [`showJoiningOff`] takes resolved text, exactly as `renderHostLobby` takes a
 * `t`: `server.html` holds a classic-script `t()` closed over `window.phStrings`
 * and the native lobby document imports `gui/strings.js` directly.
 */

/** The element whose visibility IS the join panel's visibility. */
function overlay(doc) {
  return doc.getElementById('overlay');
}

/**
 * Whether the join panel is on screen.
 *
 * Read off the element rather than off a flag — see the module note. The ground
 * state is the stylesheet's `display: none` with no inline style at all, which
 * reads as an empty string and therefore as hidden.
 */
export function isQrVisible(doc) {
  const el = overlay(doc);
  if (!el) return false;
  const shown = el.style.display;
  return shown !== '' && shown !== 'none';
}

/** Show or hide the join panel. Returns what it did, for the caller's log. */
export function setQrVisible(doc, visible) {
  const el = overlay(doc);
  if (el) el.style.display = visible ? 'block' : 'none';
  return !!visible;
}

/** Flip the join panel. Returns the new visibility. */
export function toggleQr(doc) {
  return setQrVisible(doc, !isQrVisible(doc));
}

/**
 * Carry out a phase transition's decision about the join panel.
 *
 * `action` is `hostLobbyViewModel().transitions.qrOverlayAction`. A `null`
 * action does nothing AT ALL — not "hide", not "leave it to a default": in play
 * the operator's own toggles are the only thing that moves this panel, and a
 * phase push that reasserted anything would close the QR a late arrival is
 * mid-scan of, sixty times a second.
 *
 * Returns the resulting visibility, or `null` when it declined to act.
 */
export function applyQrPhase(doc, action) {
  if (action === 'show') return setQrVisible(doc, true);
  if (action === 'hide') return setQrVisible(doc, false);
  return null;
}

/**
 * Draw one join invitation into the panel: the code, the URL under it, and the
 * the code beside them.
 *
 * @param {Document} doc the document holding the `#qr-panel` markup.
 * @param {{url: string, code?: string}} invite `url` is what the QR encodes and
 *   what the selectable text under it reads — one value, so the two can never
 *   disagree. `code` is the typed suffix a guest types instead; absent
 *   leaves that row alone.
 * @param {{toCanvas: Function}} encoder the vendored `QRCode`
 *   (`gui/vendor/qrcode.js`). Passed in rather than imported: it is a classic
 *   script that assigns a global, and passing it is also what lets a test
 *   assert the draw was reached without rasterising anything.
 * @param {{link?: boolean}} [opts] `link: false` leaves `#qr-link`'s `href`
 *   unset. The native lobby surface passes it: that document is an embedded
 *   view with no tab bar and no second window, so following the link would
 *   navigate the LOBBY away and leave the viewscreen showing a phone console.
 */
export function drawJoinQr(doc, invite, encoder, opts) {
  const url = invite && invite.url;
  if (!url) return;

  const canvas = doc.getElementById('qr');
  if (canvas && encoder && typeof encoder.toCanvas === 'function') {
    canvas.style.display = 'block';
    encoder.toCanvas(canvas, url, { width: 200 });
  }

  if (!(opts && opts.link === false)) {
    const qrLink = doc.getElementById('qr-link');
    // The href alone: what a click DOES on the host page (open a client in its
    // own window, so testers can run several side by side) is that page's own
    // business and stays in its glue.
    if (qrLink) qrLink.href = url;
  }

  const qrUrl = doc.getElementById('qr-url');
  if (qrUrl) qrUrl.textContent = url;

  if (invite.code) {
    const codeEl = doc.getElementById('join-code');
    if (codeEl) codeEl.textContent = invite.code;
    const row = doc.getElementById('join-code-row');
    if (row) row.style.display = '';
  }

  // A panel that had said "joining is off" is saying something else now.
  restoreCaption(doc);
}

/**
 * Blank the panel: no code, no URL, no letters.
 *
 * The honest response to the join service going away, and not the same as
 * hiding the panel — the letters stay wrong whether or not anybody is looking
 * at them, and a code being read aloud across a room that cannot be signalled
 * through is worse than an empty frame with a diagnostic under it.
 */
export function clearJoinQr(doc) {
  const canvas = doc.getElementById('qr');
  if (canvas) canvas.style.display = 'none';
  const qrLink = doc.getElementById('qr-link');
  if (qrLink) qrLink.removeAttribute('href');
  const qrUrl = doc.getElementById('qr-url');
  if (qrUrl) qrUrl.textContent = '';
  const codeEl = doc.getElementById('join-code');
  if (codeEl) codeEl.textContent = '';
  const row = doc.getElementById('join-code-row');
  if (row) row.style.display = 'none';
}

/**
 * Say that nobody can join this host at all.
 *
 * The native host's `--solo` (and a host given no `--rendezvous`): there is no
 * join service, so there will never be a code. A dead QR frame invites a crew
 * to stand in front of the viewscreen scanning something that cannot work, so
 * the panel says so in words instead.
 *
 * @param {string} text already resolved — `t('server.join.joining_off')`.
 */
export function showJoiningOff(doc, text) {
  clearJoinQr(doc);
  const panel = doc.getElementById('qr-panel');
  if (panel) panel.classList.add('joining-off');
  const cap = doc.getElementById('qr-caption');
  if (cap) {
    // Remembered before it is overwritten, so nothing has to know the panel's
    // ordinary caption in order to put it back.
    if (cap.dataset.caption === undefined) cap.dataset.caption = cap.textContent;
    cap.textContent = text;
  }
}

/** Undo [`showJoiningOff`]. */
function restoreCaption(doc) {
  const panel = doc.getElementById('qr-panel');
  if (panel) panel.classList.remove('joining-off');
  const cap = doc.getElementById('qr-caption');
  if (cap && cap.dataset.caption !== undefined) cap.textContent = cap.dataset.caption;
}

// Expose for the classic (non-module) script in server.html — the same
// self-registering pattern window.hostLobbyRender uses.
if (typeof window !== 'undefined') {
  window.hostQr = {
    isQrVisible, setQrVisible, toggleQr, applyQrPhase, drawJoinQr, clearJoinQr, showJoiningOff,
  };
}
