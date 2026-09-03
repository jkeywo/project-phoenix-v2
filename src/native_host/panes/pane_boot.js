// Pane boot — the FIRST script in a pane document (issue #1122).
//
// Native-side glue, not client code: it lives under src/native_host/panes/ and
// is injected into a copy of the ordinary client page, so gui/ and every
// console page stay exactly as a phone loads them.
//
// It has to be a CLASSIC script at the top of <head>, because it does three
// things the page's own scripts would otherwise get to first:
//
//   1. it seeds two values the page's own inline script reads at PARSE time and
//      computes defaults for if they are missing — the session token
//      (gui/session-token.js) and the player name. Doing this in the module half
//      would be one script too late, and the pane would join under a random name
//      on a token nothing else knows;
//   2. it NORMALISES THE URL FRAGMENT down to an ordinary join code (see below),
//      before `joinRouteFromLocation` ever reads it;
//   3. it drives `requestAnimationFrame` off a TIMER instead of the engine (see
//      below), which has to happen before `gui/bg-raf-keepalive.js` captures the
//      one it will delegate to;
//   4. it installs the page->host queue. The host polls that queue once a frame
//      with evaluate_script; there is no other channel out of an Ultralight view.
//
// WHERE THE IDENTITY COMES FROM, and why it is not in the document: the host
// binds 0.0.0.0 with no TLS and no authentication, so anything in a served body
// is readable by anything on the LAN, and a live participant's session token
// there would be a seat on the bridge handed out by `curl`. A URL FRAGMENT is
// the one part of a URL a browser never transmits — not in the request line,
// not in a header — so `pane_url` puts the token and the name there and this
// reads them back out of location.hash. See document.rs's module note.
//
// WHY THE FRAGMENT IS THEN REWRITTEN. The client page has exactly one join
// route, and the fragment is its input: `joinRouteFromLocation` reads ANY
// non-empty fragment as a rendezvous code and hands it to `parseJoinCode`,
// which refuses `token=…&name=…` and puts the join-entry overlay over the
// console. A pane is not exempt from that route and must not try to be — it
// joins through the page's ordinary `startPhoenixJoin`, over the in-process
// transport pane_link.js installs. So the fragment carries BOTH: a well-formed
// typed join code first, then the identity fields. This script consumes
// the identity and leaves the code, and from that line on the page's URL is
// indistinguishable from a phone's that was handed a code — which is exactly
// what it then behaves like. It also means the pane's own session token has
// stopped being readable from `location.hash` by the time any page code runs.
(function () {
  // Operator-profile capability declaration (issue #1280). The pane still
  // loads the ordinary client page and its one versioned profile; this tells
  // that shared adapter which retained settings can be active here. Keyboard
  // input is delivered by #1124's focused native input route. Ultralight has no
  // Gamepad API sampling or vibration backend, so neither gets a second native
  // route. The profile keeps both choices for later export to a capable browser.
  window.PhoenixOperatorCapabilities = Object.freeze({
    surface: 'native-pane',
    keyboard: true,
    gamepad: false,
    vibration: false,
    semanticCues: true,
    accessibility: true,
  });

  // ── requestAnimationFrame, off a timer ─────────────────────────────────────
  //
  // NOT a nicety, and not about smoothness. An offscreen Ultralight view
  // services rAF callbacks as part of a *rendering update*, and it only runs one
  // when something has made the page dirty. The client page's whole render loop
  // is `scheduleRender()`:
  //
  //     if (_renderFrame !== null) return;
  //     _renderFrame = requestAnimationFrame(() => { _renderFrame = null; render(…); });
  //
  // so a pane that reached a quiet moment — state settled, nothing animating,
  // the next repaint owed entirely to a render nobody has drawn yet — deadlocks:
  // no rendering update, so the callback never fires; the callback never fires,
  // so nothing mutates the DOM; nothing mutates the DOM, so there is no
  // rendering update. `_renderFrame` stays non-null, every later
  // `scheduleRender()` returns at the first line, and the page is frozen at its
  // last paint FOREVER while its transport keeps delivering perfectly good
  // state. Observed as roughly one console in four coming up with the lobby
  // still over it, holding a Station it knew it held.
  //
  // Timers are not starved this way — `Renderer::update()` runs them whether or
  // not anything painted, which is why the page's own 500 ms name-field debounce
  // fired in exactly the runs whose rAF never did. So a pane's rAF is a timer.
  //
  // It has to be installed HERE, before `gui/bg-raf-keepalive.js` evaluates,
  // because that module captures `window.requestAnimationFrame` once and
  // delegates to it whenever the document is visible (which a pane always is).
  // Replacing it afterwards would leave that captured reference in charge.
  var FRAME_MS = 16;
  try {
    window.requestAnimationFrame = function (cb) {
      return setTimeout(function () {
        cb(window.performance && window.performance.now ? window.performance.now() : Date.now());
      }, FRAME_MS);
    };
    window.cancelAnimationFrame = function (id) {
      clearTimeout(id);
    };
  } catch (e) {
    // A document that will not take the override renders on the engine's own
    // terms; that is the behaviour this replaces, not something worse.
  }

  // `#<CODE>&token=<pct>&name=<pct>`. The one part with no `=` is the join
  // code (document.rs's PANE_JOIN_CODE); everything else is a pair.
  var identity = { token: '', name: '' };
  var code = '';
  var raw = '';
  try {
    raw = String(window.location.hash || '').replace(/^#/, '');
  } catch (e) {
    // A document with no location is not a pane; the transport pane_link.js
    // installs still pins the identity it was given, and the host refuses an
    // empty token.
  }
  var parts = raw.split('&');
  for (var p = 0; p < parts.length; p++) {
    var eq = parts[p].indexOf('=');
    if (eq < 0) {
      if (!code) code = parts[p];
      continue;
    }
    var key = parts[p].slice(0, eq);
    if (key !== 'token' && key !== 'name') continue;
    try {
      identity[key] = decodeURIComponent(parts[p].slice(eq + 1));
    } catch (e) {
      // A malformed escape in one field must not cost us the other.
    }
  }

  try {
    // gui/session-token.js reads this key; seeding it is what makes the page
    // present the token the host minted rather than one of its own.
    sessionStorage.setItem('session-token', identity.token);
    sessionStorage.setItem('player-name', identity.name);
  } catch (e) {
    // Storage can be unavailable. The pane transport pins both values on the
    // Identify it forwards, so this is the ordinary path rather than the only
    // one.
  }

  // Hand the page a fragment it can actually join with, and take the identity
  // out of the URL while we are here. `replaceState` leaves no history entry;
  // assigning `location.hash` is the fallback and navigates within the document
  // rather than reloading it. If both fail the page will show its join-entry
  // field, which is the honest outcome — a pane whose route could not be set is
  // not silently half-joined.
  if (code) {
    try {
      if (window.history && typeof window.history.replaceState === 'function') {
        window.history.replaceState(null, '', '#' + code);
      } else {
        window.location.hash = code;
      }
    } catch (e) {
      try {
        window.location.hash = code;
      } catch (e2) {
        /* nothing left to try */
      }
    }
  }

  // How many undelivered messages the page may hold before it tells the host to
  // stop. Reached only when the page has stopped draining — the pane transport
  // installs `deliver` as soon as the page's own joiner is up, and from then on
  // every push drains immediately — so this is a wedged-page bound, not a
  // throughput one.
  //
  // It is what makes the RELIABLE-overflow close reachable at all. The host's
  // own cap (registry.rs) only sees messages it has not handed over yet, and a
  // page that accepts every push and does nothing with it never lets that queue
  // grow. Throwing here turns the next push into a `PaneSurfaceError::Script`,
  // which pump_pane requeues; the host queue then fills, overflows its reliable
  // budget, and drive_panes closes the pane — the same disconnect a dropped
  // phone produces, so the station falls back to AI control.
  var INBOX_CAP = 256;

  var pane = {
    token: identity.token,
    name: identity.name,
    // The join code the host put in the fragment, kept so pane_link.js can say
    // what it is standing in for and a test can assert the two halves agree.
    code: code,
    // Messages that arrived before the page's joiner was up. The host starts
    // pushing as soon as the document reports it has loaded, and 'loaded' is
    // not 'the page has joined'.
    inbox: [],
    deliver: null,
  };
  pane.drainInbox = function () {
    if (!pane.deliver) return;
    var batch = pane.inbox.splice(0);
    for (var i = 0; i < batch.length; i++) pane.deliver(batch[i]);
  };
  window.__phoenixPane = pane;

  // Host -> page. Called by the host once per queued ServerMessage.
  window.__phoenixPaneApply = function (json) {
    if (pane.inbox.length >= INBOX_CAP) {
      // Deliberately a throw rather than a dropped message: the host is the
      // one that knows which messages may be dropped (snapshots) and which may
      // not (Welcome, StationAssigned, GameStarted), and it cannot make that
      // call for a message the page silently swallowed.
      throw new Error(
        '[pane] ' + pane.inbox.length + ' undelivered messages: the page is not draining'
      );
    }
    pane.inbox.push(json);
    pane.drainInbox();
  };
})();
