/**
 * gui/page-chrome.js — shared page chrome for server.html and client.html
 * (issue #1227): the connection-status dot, the Screen Wake Lock (plus its
 * `visibilitychange` re-acquire), the fullscreen toggle, and the mechanical
 * half of the connection-diagnostics readout under `#conn-diag`. Both pages
 * used to carry their own near-identical copies of all four; this module is
 * the single source.
 *
 * Plain DOM/Web API code with no free-standing `window`/`document`
 * reference beyond what a caller passes in (or the real globals when it
 * doesn't), so it imports cleanly under vitest with `@vitest-environment
 * jsdom` — see tests/client/page-chrome.test.js.
 *
 * ## Boundary bridging
 *
 * server.html's chrome lives in a CLASSIC script and cannot `import` this
 * module directly. client.html turns out to have the exact same shape —
 * its bulk logic is also a classic script, not a module, despite the page
 * otherwise being built from `<script type="module">` islands. Both pages
 * mount this module through the same latch/queue bridge the #1224 host
 * channel and #1225/#1226 islands established: a `window.__pageChromeReady`
 * callback the module island calls once mounted, with a small queue so any
 * classic-script call made before that (server.html calls
 * `setConnectionStatus('connecting')`/`acquireWakeLock()` synchronously at
 * parse time, before any deferred module has run) is replayed in order
 * rather than dropped. See the classic-script bridge block in both pages,
 * just above their `function t(id, params)` bridge.
 *
 * ## Real per-page differences kept as mount-time options, not papered over
 *
 *  - server.html's status dot used hardcoded hex colours (`#4c4`/`#f44`);
 *    client.html's used the shared CSS custom properties from
 *    gui/tokens.css (`--loaded`/`--fire-hot`), which resolve to DIFFERENT
 *    literal colours (see gui/tokens.css). Passed in as `statusColors`.
 *  - the disconnected/error string ids differ per page (`server.*` vs
 *    `client.*`) because the copy itself differs. Passed in as
 *    `statusLabels`.
 *  - only client.html has a `#retry-now-btn` to show/hide while the link is
 *    down; guarded by presence here, so it is a no-op on server.html, which
 *    has no such button.
 *  - only server.html released the wake lock on `beforeunload`;
 *    client.html never registered that listener. Opt in per page with
 *    `releaseWakeLockOnUnload`.
 *
 * ## What is deliberately NOT unified
 *
 * server.html's `renderConnDiag` summarised multi-client ICE stages across
 * every connected phone (host-side, keyed by peer id, driven by PeerJS
 * connection events). client.html's `renderDiag` summarised this one
 * device's own relay probe verdict plus its own ICE attempt/candidate
 * history (client-side, keyed by attempt number, driven by
 * connection-manager's `onDiag` callback). The data shapes and every
 * string id involved are unrelated — forcing them into one function would
 * either lose information or invent a shared schema neither page actually
 * has. Only the shared mechanical bit — guard for a missing element, join
 * lines, set `textContent` — is extracted here as `mountConnDiag()`; each
 * page still builds its own `lines` array from its own state and hands it
 * to the renderer this returns.
 */

/**
 * Mount the shared page chrome onto `doc`.
 *
 * @param {{
 *   doc?: Document,
 *   t: (id: string, params?: Record<string, string|number>) => string,
 *   statusColors?: { active?: string, trouble?: string },
 *   statusLabels?: { disconnected?: string, error?: string },
 *   releaseWakeLockOnUnload?: boolean,
 * }} opts
 * @returns {{
 *   setConnectionStatus: (state: string) => void,
 *   acquireWakeLock: () => Promise<void>,
 *   releaseWakeLock: () => void,
 *   mountConnDiag: (elId?: string) => (lines: string[]) => void,
 * }}
 */
export function mountPageChrome({
  doc = document,
  t,
  statusColors = {},
  statusLabels = {},
  releaseWakeLockOnUnload = false,
} = {}) {
  const activeColor = statusColors.active ?? '#4c4';
  const troubleColor = statusColors.trouble ?? '#f44';
  const disconnectedLabel = statusLabels.disconnected ?? 'client.reconnecting';
  const errorLabel = statusLabels.error ?? 'server.conn_error_refresh';

  // ── Connection status dot ────────────────────────────────────────────

  function setConnectionStatus(state) {
    const dot = doc.getElementById('conn-dot');
    const label = doc.getElementById('conn-label');
    if (!dot || !label) return;
    // Only client.html has this button — a no-op everywhere else.
    const retryBtn = doc.getElementById('retry-now-btn');
    if (state === 'disconnected') {
      dot.style.background = troubleColor;
      label.textContent = t(disconnectedLabel);
      if (retryBtn) retryBtn.classList.add('visible');
    } else if (state === 'error') {
      dot.style.background = troubleColor;
      label.textContent = t(errorLabel);
      if (retryBtn) retryBtn.classList.add('visible');
    } else {
      dot.style.background = activeColor;
      label.textContent = '';
      if (retryBtn) retryBtn.classList.remove('visible');
    }
  }

  // ── Screen Wake Lock (keep screen on while active) ──────────────────

  let wakeLockSentinel = null;

  async function acquireWakeLock() {
    try {
      if (wakeLockSentinel) return;
      wakeLockSentinel = await navigator.wakeLock.request('screen');
      wakeLockSentinel.addEventListener('release', () => { wakeLockSentinel = null; });
    } catch (_) {}
  }

  function releaseWakeLock() {
    if (wakeLockSentinel) { wakeLockSentinel.release(); wakeLockSentinel = null; }
  }

  doc.addEventListener('visibilitychange', () => {
    if (doc.visibilityState === 'visible') acquireWakeLock();
  });

  if (releaseWakeLockOnUnload) {
    window.addEventListener('beforeunload', () => releaseWakeLock());
  }

  // ── Fullscreen controller ────────────────────────────────────────────

  (function initFullscreen() {
    const btn = doc.getElementById('fullscreen-btn');
    if (!btn) return;
    function syncIcon() { btn.textContent = doc.fullscreenElement ? '✕' : '⛶'; }
    btn.addEventListener('click', () => {
      const p = doc.fullscreenElement
        ? doc.exitFullscreen()
        : doc.documentElement.requestFullscreen();
      p && p.catch(() => {});
    });
    doc.addEventListener('fullscreenchange', syncIcon);
  })();

  // ── Connection diagnostics (mechanical half only — see module doc) ──

  function mountConnDiag(elId = 'conn-diag') {
    const el = doc.getElementById(elId);
    return function renderConnDiag(lines) {
      if (!el) return;
      el.textContent = lines.join('\n');
    };
  }

  return { setConnectionStatus, acquireWakeLock, releaseWakeLock, mountConnDiag };
}
