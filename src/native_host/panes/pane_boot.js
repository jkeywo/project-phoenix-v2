// Pane boot — the FIRST script in a pane document (issue #1122).
//
// Native-side glue, not client code: it lives under src/native_host/panes/ and
// is injected into a copy of the ordinary client page, so gui/ and every
// console page stay exactly as a phone loads them.
//
// It has to be a CLASSIC script at the top of <head>, because it seeds two
// values the page's own inline script reads at parse time — before any module
// runs — and computes defaults for if they are missing: the session token
// (gui/session-token.js) and the player name. Doing this in the module half
// would be one script too late, and the pane would join under a random name on
// a token nothing else knows.
//
// It also installs the page->host queue. The host polls that queue once a frame
// with evaluate_script; there is no other channel out of an Ultralight view.
//
// WHERE THE IDENTITY COMES FROM, and why it is not in the document: the host
// binds 0.0.0.0 with no TLS and no authentication, so anything in a served body
// is readable by anything on the LAN, and a live participant's session token
// there would be a seat on the bridge handed out by `curl`. A URL FRAGMENT is
// the one part of a URL a browser never transmits — not in the request line,
// not in a header — so `pane_url` puts the token and the name there and this
// reads them back out of location.hash. See document.rs's module note.
(function () {
  // `#native&token=<pct>&name=<pct>`. The `native` prefix with no `=` is the
  // page's own join route (see document.rs); everything else is a pair.
  var identity = { token: '', name: '' };
  var raw = '';
  try {
    raw = String(window.location.hash || '').replace(/^#/, '');
  } catch (e) {
    // A document with no location is not a pane; the link script below still
    // sends whatever it has, and the host refuses an empty token.
  }
  var parts = raw.split('&');
  for (var p = 0; p < parts.length; p++) {
    var eq = parts[p].indexOf('=');
    if (eq < 0) continue;
    var key = parts[p].slice(0, eq);
    if (key !== 'token' && key !== 'name') continue;
    try {
      // The host escapes '_' as %5F so the fragment cannot look like a
      // rendezvous join code; decodeURIComponent puts it back.
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
    // Storage can be unavailable. The link script pins both values directly
    // when it sends Identify, so this is a convenience rather than the path.
  }

  // How many undelivered messages the page may hold before it tells the host to
  // stop. Reached only when the page has stopped draining — the link script
  // installs `deliver` once its modules have run, and from then on every push
  // drains immediately — so this is a wedged-page bound, not a throughput one.
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
    // Messages that arrived before the link script installed its delivery
    // function. The host starts pushing as soon as the document reports it has
    // loaded, and 'loaded' is not 'every module has run'.
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
