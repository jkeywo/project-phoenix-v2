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
// window.__phoenixPaneIdentity is prepended by the host immediately above this.
(function () {
  var identity = window.__phoenixPaneIdentity || { token: '', name: '' };

  try {
    // gui/session-token.js reads this key; seeding it is what makes the page
    // present the token the host minted rather than one of its own.
    sessionStorage.setItem('session-token', identity.token);
    sessionStorage.setItem('player-name', identity.name);
  } catch (e) {
    // Storage can be unavailable. The link script pins both values directly
    // when it sends Identify, so this is a convenience rather than the path.
  }

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
    pane.inbox.push(json);
    pane.drainInbox();
  };
})();
