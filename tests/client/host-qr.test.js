// @vitest-environment jsdom
//
// tests/client/host-qr.test.js — the shared join panel (issue #1329).
//
// gui/host-qr.js is the one place the join QR is drawn and the one place its
// visibility is decided, for BOTH hosts: the browser page, and the document the
// native host composites onto its viewscreen. Three things are worth pinning:
//
//   1. the visibility law — shown in the lobby, hidden at mission start, and
//      LEFT ALONE in play, which is what lets a late arrival's code stay up;
//   2. the writes land in the real markup (the fixture is server.html's own
//      #overlay subtree, not a hand-typed copy that could drift from it);
//   3. the browser host's draw is still REACHED. That last one is this issue's
//      AC5: the draw used to sit inside PeerJS's `peer.on('open')` callback,
//      #1112 deleted PeerJS, and nothing since then has been checking that its
//      replacement is on the boot path at all.
//
// The native document's half of (2) and (3) is asserted in Rust, where that
// document is assembled — `native_host::host_lobby::document`'s tests.
import { describe, it, expect, beforeEach } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { hostLobbyViewModel } from '../../gui/host-lobby-view.js';
import {
  isQrVisible, setQrVisible, toggleQr, applyQrPhase,
  drawJoinQr, clearJoinQr, showJoiningOff,
} from '../../gui/host-qr.js';
import { joinUrlForCode } from '../../gui/join-url.js';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
const SERVER_HTML = fs.readFileSync(path.join(ROOT, 'server.html'), 'utf-8');
/** The native lobby document's glue, read as source for the same reasons. */
const LINK_JS = fs.readFileSync(
  path.join(ROOT, 'src/native_host/host_lobby/host_lobby_link.js'), 'utf-8',
);

/**
 * The body of one named function in server.html's classic script, and NOTHING
 * after it.
 *
 * Scanning the whole file with a non-greedy `[\s\S]*?` between a function's
 * header and the call it is supposed to make does not pin that call: the match
 * runs happily past the end of the function to the next occurrence anywhere
 * below, so the assertion passes with the call deleted. Both landing
 * transitions were pinned that way and both passed against a mutant that had
 * lost the call — to `showJoinQrOverPanel`'s copy of it, forty lines further
 * down. Every function in that script closes on a `    }` at its own indent,
 * which is where the body ends.
 */
function serverFunctionBody(name) {
  const start = SERVER_HTML.indexOf(`function ${name}()`);
  if (start < 0) throw new Error(`server.html has no function ${name}()`);
  const end = SERVER_HTML.indexOf('\n    }', start);
  if (end < 0) throw new Error(`function ${name}() is never closed`);
  return SERVER_HTML.slice(start, end);
}

/** server.html between two landmarks, so a scan cannot run past the second. */
function between(from, to) {
  const start = SERVER_HTML.indexOf(from);
  const end = SERVER_HTML.indexOf(to, start + 1);
  if (start < 0 || end < 0) throw new Error(`server.html has no ${from} … ${to}`);
  return SERVER_HTML.slice(start, end);
}

/** The settings cog's toggle seam, `window.__hostToggleQrCode`, on its own. */
function hostToggleBody() {
  return between('window.__hostToggleQrCode = function', 'window.__hostIsQrVisible');
}

/** The same, for the native document's top-level functions (closed at `\n}`). */
function linkFunctionBody(header) {
  const start = LINK_JS.indexOf(header);
  if (start < 0) throw new Error(`host_lobby_link.js has no ${header}`);
  const end = LINK_JS.indexOf('\n}', start);
  if (end < 0) throw new Error(`${header} is never closed`);
  return LINK_JS.slice(start, end);
}

/** server.html's real #overlay subtree, lifted into the test document. */
function installJoinPanel(doc) {
  const parsed = new DOMParser().parseFromString(SERVER_HTML, 'text/html');
  const overlay = parsed.getElementById('overlay');
  if (!overlay) throw new Error('#overlay not found in server.html');
  doc.body.appendChild(doc.importNode(overlay, true));
  return doc.getElementById('overlay');
}

/** A `QRCode` stand-in that records what it was asked to draw. */
function recordingEncoder() {
  const draws = [];
  return { draws, toCanvas: (canvas, text, opts) => { draws.push({ canvas, text, opts }); } };
}

const INVITE = {
  url: 'http://192.168.1.5:8080/client/index.html#PHX-1-ABCDE',
  code: 'ABCDE',
};

/** The lobby payload shape `hostLobbyViewModel` consumes. */
const payload = (phase) => ({
  phase, scenario_title: 'Combat Test', scenario_body: '', crew_count: 0,
  max_players: 2, all_ready: false, stations: [], spectators: [], countdown_secs: 0,
});

beforeEach(() => {
  document.body.innerHTML = '';
  installJoinPanel(document);
});

describe('the visibility law', () => {
  it('starts hidden, because the stylesheet is the ground state', () => {
    // No inline style at all: gui/host-qr.css says `display: none` and nothing
    // has spoken yet. A module-level `visible = false` would have agreed here
    // and then drifted the first time anything else wrote the element.
    expect(isQrVisible(document)).toBe(false);
  });

  it('shows the panel in the lobby phase', () => {
    const vm = hostLobbyViewModel(payload('Lobby'), '');
    expect(vm.transitions.qrOverlayAction).toBe('show');
    applyQrPhase(document, vm.transitions.qrOverlayAction);
    expect(isQrVisible(document)).toBe(true);
  });

  it('hides it while the mission loads and once it is over', () => {
    for (const phase of ['Loading', 'GameOver']) {
      setQrVisible(document, true);
      const vm = hostLobbyViewModel(payload(phase), 'Lobby');
      expect(vm.transitions.qrOverlayAction).toBe('hide');
      applyQrPhase(document, vm.transitions.qrOverlayAction);
      expect(isQrVisible(document)).toBe(false);
    }
  });

  it('leaves it exactly as it was during a running mission', () => {
    // The load-bearing null. A phase push that reasserted anything here would
    // shut the QR an operator opened for a late arrival — and it arrives every
    // frame the lobby state changes, so it would shut it repeatedly.
    const vm = hostLobbyViewModel(payload('InProgress'), 'InProgress');
    expect(vm.transitions.qrOverlayAction).toBe(null);

    setQrVisible(document, true);
    expect(applyQrPhase(document, vm.transitions.qrOverlayAction)).toBe(null);
    expect(isQrVisible(document)).toBe(true);

    setQrVisible(document, false);
    applyQrPhase(document, vm.transitions.qrOverlayAction);
    expect(isQrVisible(document)).toBe(false);
  });

  it('stays off the landing, and comes back with the lobby', () => {
    // The reported bug: `GamePhase::Lobby` is the default, so a world-less host
    // reads "Lobby" from its first frame and the phase law alone put the join
    // code on top of the landing (the native overlay is z-index 210 over the
    // landing's 205). The landing is a pre-simulation surface with no phase of
    // its own, so it is a second input rather than something inferred here.
    //
    // `hide` and not `null`, because the host page docks this same node into
    // #scenario-panel at page load and can have shown it before any lobby
    // payload arrives.
    setQrVisible(document, true);
    const onLanding = hostLobbyViewModel(payload('Lobby'), '', null, { landingUp: true });
    expect(onLanding.transitions.qrOverlayAction).toBe('hide');
    applyQrPhase(document, onLanding.transitions.qrOverlayAction);
    expect(isQrVisible(document)).toBe(false);

    const inLobby = hostLobbyViewModel(payload('Lobby'), 'Lobby', null, { landingUp: false });
    applyQrPhase(document, inLobby.transitions.qrOverlayAction);
    expect(isQrVisible(document)).toBe(true);
  });

  it('shows on a landing that is CARRYING it — issue #755 AC1', () => {
    // The other half of the landing rule, and the reason it is two facts and
    // not one. The host page docks this very node into the World picker
    // (`#overlay.pre-scenario`), which the landing borrows into its middle
    // column: a panel the landing carries cannot cover it, and a crew scans the
    // code from the same column the room is picking a World in.
    //
    // The phase is not consulted on either landing row, which matters most
    // here: no world is loaded, so this page has had no lobby payload and its
    // `_lobbyPrevPhase` is still `''` — a fall-through to the phase law would
    // answer `null` and leave the code hidden for the whole of selection.
    setQrVisible(document, false);
    const docked = hostLobbyViewModel(payload('Lobby'), '', null, {
      landingUp: true, panelDocked: true,
    });
    expect(docked.transitions.qrOverlayAction).toBe('show');
    applyQrPhase(document, docked.transitions.qrOverlayAction);
    expect(isQrVisible(document)).toBe(true);

    expect(hostLobbyViewModel(payload(''), '', null, {
      landingUp: true, panelDocked: true,
    }).transitions.qrOverlayAction).toBe('show');
  });

  it('toggles from whatever is on screen, whoever asked', () => {
    // The cog, a phone's ToggleQrCode and the native surface's own control all
    // land here, and none of them carries a "should be" of its own.
    expect(toggleQr(document)).toBe(true);
    expect(isQrVisible(document)).toBe(true);
    expect(toggleQr(document)).toBe(false);
    expect(isQrVisible(document)).toBe(false);
  });

  it('survives a document with no join panel at all', () => {
    document.body.innerHTML = '';
    expect(() => applyQrPhase(document, 'show')).not.toThrow();
    expect(isQrVisible(document)).toBe(false);
  });
});

describe('drawing an invitation', () => {
  it('encodes the join URL and writes the same one under it', () => {
    const encoder = recordingEncoder();
    drawJoinQr(document, INVITE, encoder);

    expect(encoder.draws).toHaveLength(1);
    expect(encoder.draws[0].text).toBe(INVITE.url);
    expect(encoder.draws[0].canvas).toBe(document.getElementById('qr'));
    // One value drives both, so the code and the text under it cannot disagree
    // — which is the whole point of the readable URL being there.
    expect(document.getElementById('qr-url').textContent).toBe(INVITE.url);
    expect(document.getElementById('qr').style.display).toBe('block');
  });

  it('shows the code a guest types instead of scanning', () => {
    drawJoinQr(document, INVITE, recordingEncoder());
    expect(document.getElementById('join-code').textContent).toBe('ABCDE');
    expect(document.getElementById('join-code-row').style.display).toBe('');
  });

  it('links the QR for a page with somewhere to open it', () => {
    drawJoinQr(document, INVITE, recordingEncoder());
    expect(document.getElementById('qr-link').getAttribute('href')).toBe(INVITE.url);
  });

  it('leaves the link unset for the native surface', () => {
    // An embedded lobby view has no tab bar and no second window: following
    // the link would navigate the LOBBY away and leave the viewscreen showing
    // a phone console with no way back.
    drawJoinQr(document, INVITE, recordingEncoder(), { link: false });
    expect(document.getElementById('qr-link').hasAttribute('href')).toBe(false);
    // …and the code itself is still drawn. The point is the destination, not
    // the QR.
    expect(document.getElementById('qr-url').textContent).toBe(INVITE.url);
  });

  it('draws nothing at all for an empty invitation', () => {
    const encoder = recordingEncoder();
    drawJoinQr(document, { url: '' }, encoder);
    expect(encoder.draws).toHaveLength(0);
  });

  it('blanks the panel when the join service goes away', () => {
    drawJoinQr(document, INVITE, recordingEncoder());
    clearJoinQr(document);
    // A code being read aloud across a room that cannot be signalled through
    // is worse than an empty frame.
    expect(document.getElementById('qr-url').textContent).toBe('');
    expect(document.getElementById('join-code').textContent).toBe('');
    expect(document.getElementById('qr').style.display).toBe('none');
    expect(document.getElementById('qr-link').hasAttribute('href')).toBe(false);
  });
});

describe('a host nobody can join', () => {
  it('says so instead of framing a dead QR', () => {
    // `phoenix-host --solo`, or one given no `--rendezvous`: there is no join
    // service, so there will never be a code, and a crew should not be stood
    // in front of the viewscreen scanning something that cannot work.
    showJoiningOff(document, '[Crew joining is off]');
    expect(document.getElementById('qr-caption').textContent).toBe('[Crew joining is off]');
    expect(document.getElementById('qr-panel').classList.contains('joining-off')).toBe(true);
    expect(document.getElementById('qr-url').textContent).toBe('');
    expect(document.getElementById('qr').style.display).toBe('none');
  });

  it('puts its own caption back if a code ever does arrive', () => {
    const original = document.getElementById('qr-caption').textContent;
    showJoiningOff(document, '[Crew joining is off]');
    drawJoinQr(document, INVITE, recordingEncoder());
    expect(document.getElementById('qr-caption').textContent).toBe(original);
    expect(document.getElementById('qr-panel').classList.contains('joining-off')).toBe(false);
  });
});

describe('where a native host QR sends a phone', () => {
  // The equality this issue's AC1 turns on: what the QR encodes has to BE the
  // join URL a phone needs. The physical scan belongs to the #1335 kit; this is
  // the half a test can hold.
  //
  // What the boundary actually is, so this is not read as more than it is:
  // Rust supplies only `page_base` (`native_host::host_lobby::join` decides
  // which address a phone is sent to, and tests that decision on its own side);
  // gui/join-url.js is the SOLE builder of the URL, for this host and the
  // browser one alike. So the value below is a stand-in for what crosses, not a
  // cross-boundary pin — the real end-to-end proof is the ignored SDK test
  // (tests/native_host_lobby_ultralight.rs), which drives the whole chain
  // against a real window. What this block does hold is the half that matters
  // most and needs no GPU: given a page base, the URL is the client page beside
  // the host bundle, and the QR and the printed text carry one value.
  const PAGE_BASE = 'http://192.168.1.5:8080/';

  it('is the client page beside the host bundle, with the code in the fragment', () => {
    expect(joinUrlForCode(PAGE_BASE, 'PHX-1-ABCDE'))
      .toBe('http://192.168.1.5:8080/client/index.html#PHX-1-ABCDE');
  });

  it('names a non-default service, so the phone dials the one the host registered with', () => {
    expect(joinUrlForCode(PAGE_BASE, 'PHX-1-ABCDE', 'http://127.0.0.1:8788')).toBe(
      'http://192.168.1.5:8080/client/index.html'
        + '?rendezvous=http%3A%2F%2F127.0.0.1%3A8788#PHX-1-ABCDE',
    );
  });

  it('is exactly what the panel encodes and prints', () => {
    // The other half of the equality: the URL the QR carries and the URL the
    // selectable text carries are one value, so a guest who cannot scan and a
    // guest who can end up in the same place.
    const encoder = recordingEncoder();
    const url = joinUrlForCode(PAGE_BASE, 'PHX-1-ABCDE');
    drawJoinQr(document, { url, code: 'ABCDE' }, encoder, { link: false });
    expect(encoder.draws[0].text).toBe(url);
    expect(document.getElementById('qr-url').textContent).toBe(url);
  });
});

describe('server.html still reaches the draw (issue #1329 AC5)', () => {
  // The regression this issue was asked to re-verify. The join QR used to be
  // drawn inside PeerJS's `peer.on('open')` callback; #1112 deleted PeerJS and
  // re-homed the draw on the rendezvous service's `onCode`. It was already
  // correct — this is what makes it stay correct, by pinning the whole chain
  // from the callback the transport invokes down to the shared module.

  it('draws from the rendezvous code callback, not a transport that is gone', () => {
    expect(SERVER_HTML).toContain('onCode: (code) => showJoinCode(code)');
    expect(SERVER_HTML).toMatch(/function showJoinCode\(code\)\s*\{[\s\S]*?paintJoinPanel\(/);
    expect(SERVER_HTML).toMatch(
      /function paintJoinPanel\(url, code\)\s*\{[\s\S]*?hostQr\.drawJoinQr\(/,
    );
    // Nothing is left of the old home. The `*`-prefixed lines are the comment
    // that says where the draw used to be and why it is not there any more —
    // worth keeping, and not a call site.
    expect(SERVER_HTML).not.toMatch(/^(?!\s*\*).*peer\.on\(/m);
    expect(SERVER_HTML).not.toContain('new Peer(');
  });

  it('has exactly one join-QR draw site, and it is the shared module', () => {
    // A second `QRCode.toCanvas` against #qr would be a second rendering path
    // — the thing this issue exists to prevent. The fleet panel's own canvas
    // (#fleet-qr, issue #1114) is a different panel for a different audience
    // and is deliberately not counted.
    const joinDraws = SERVER_HTML.match(/QRCode\.toCanvas\(\s*canvas/g) || [];
    expect(joinDraws).toHaveLength(0);
    expect(SERVER_HTML).toContain('<script type="module" src="gui/host-qr.js"></script>');
  });

  it('asks the shared law at every moment it moves the panel without a payload', () => {
    // The pre-scenario dock (issue #755) used to end in an unconditional
    // `setQrVisible(document, true)`. That was right while the picker was the
    // first screen and wrong once the landing (PRD #1355) stood in front of it:
    // #scenario-panel is moved into #landing-mid when New Game opens, and the
    // docked QR rode along and showed during World selection. The docking is
    // still this page's; the visibility is the law's.
    const dock = serverFunctionBody('showJoinQrOverPanel');
    expect(dock).toContain('applyJoinPanelLaw();');
    expect(dock).not.toContain('hostQr.setQrVisible');
    expect(SERVER_HTML).not.toContain('hostQr.setQrVisible(document, true)');
    // …and the law itself is the shared module's, reached with the facts this
    // page holds — never re-derived here. `joinPanelDockedInLanding()` is the
    // third: without it the docked panel falls to the phase law, which reads
    // `''` before any world is loaded, and issue #755's AC1 goes quietly out.
    const law = serverFunctionBody('applyJoinPanelLaw');
    expect(law).toContain('window.joinPanelAction(');
    expect(law).toContain(
      '_lobbyPrevPhase, !_landingDismissed, joinPanelDockedInLanding(),',
    );
    // Both landing transitions re-ask it, AFTER writing the fact they moved:
    // the lobby push is deduped, so "the landing went away" would otherwise
    // never reach the panel — and asking before the write would answer about
    // the landing that has just gone.
    const gone = serverFunctionBody('hideLanding');
    expect(gone).toContain('applyJoinPanelLaw();');
    expect(gone.indexOf('_landingDismissed = true'))
      .toBeLessThan(gone.indexOf('applyJoinPanelLaw()'));
    const back = serverFunctionBody('showLandingAtPicker');
    expect(back).toContain('applyJoinPanelLaw();');
    expect(back.indexOf('_landingDismissed = false'))
      .toBeLessThan(back.indexOf('applyJoinPanelLaw()'));
    // And the lobby payload path hands the same facts to the view model.
    expect(SERVER_HTML).toMatch(/hostLobbyViewModel\([\s\S]{0,160}?landingUp: !_landingDismissed/);
    expect(SERVER_HTML).toMatch(
      /hostLobbyViewModel\([\s\S]{0,200}?panelDocked: joinPanelDockedInLanding\(\)/,
    );
  });

  it('refuses a toggle while the landing stands in front of the panel', () => {
    // The table in gui/host-qr.js says the first row that matches wins, and a
    // toggle that ignored it would win instead: nothing on either host
    // reasserts the law during a mission, the lobby push is deduped, and the
    // landing only re-asks when it MOVES — so one press over the front door
    // leaves the join code sitting on it.
    //
    // Both of this page's entry points, and the refusal is the shared module's
    // `joinPanelSuppressed` rather than a rule re-stated here. A phone's press
    // arrives as a MESSAGE, so hiding a control would not have covered it.
    const cog = hostToggleBody();
    expect(cog).toContain('joinPanelBlockedByLanding()');
    expect(cog.indexOf('joinPanelBlockedByLanding()'))
      .toBeLessThan(cog.indexOf('hostQr.toggleQr(document)'));
    const phone = between("parsed.type === 'ToggleQrCode'", "parsed.type === 'SelectScenario'");
    expect(phone).toContain('joinPanelBlockedByLanding()');
    expect(phone.indexOf('joinPanelBlockedByLanding()'))
      .toBeLessThan(phone.indexOf('hostQr.toggleQr(document)'));
    const blocked = serverFunctionBody('joinPanelBlockedByLanding');
    expect(blocked).toContain(
      'window.joinPanelSuppressed(!_landingDismissed, joinPanelDockedInLanding())',
    );
  });

  it('never docks the panel, so “landing up” is the whole of its answer', () => {
    // The two surfaces differ in exactly one fact, and this is it: the host
    // page parks #overlay inside the World picker, while this document leaves
    // it floating at z-index 210 over the landing's 205. Passing `panelDocked`
    // here would put the join code back over the front door — the reported bug.
    expect(LINK_JS).not.toMatch(/panelDocked:/);
    expect(LINK_JS).not.toContain('pre-scenario');
  });

  it('keeps no second notion of whether the panel is on screen', () => {
    // The `qrVisible` flag four handlers used to keep in step. The state is
    // #overlay, and gui/host-qr.js is the only thing that reads or writes it.
    expect(SERVER_HTML).not.toMatch(/^\s*let qrVisible/m);
    // …and the settings cog's two entry points really do land on the module
    // rather than on a flag of their own. This is the seam the deleted flag
    // used to sit behind: `__hostToggleQrCode` set `display` and flipped
    // `qrVisible`, and mission start set `display` WITHOUT flipping it, so the
    // first cog press after a launch was a no-op the operator had to press
    // twice. Reading the DOM is what makes that unrepresentable.
    expect(hostToggleBody()).toContain('hostQr.toggleQr(');
    expect(SERVER_HTML).toMatch(/__hostIsQrVisible[\s\S]{0,120}?hostQr\.isQrVisible\(/);
  });
});

// The native lobby document's glue is the OTHER caller of the same law, and the
// one whose defaults are wrong in a way a green suite would not notice: it is
// the surface the bug was reported on, and the configuration that breaks is the
// one nobody runs by hand. server.html has had a source scan since #1329; this
// is its twin.
describe('the native lobby document reaches the same law', () => {
  it('reads “the landing is up” as a landing that was PUSHED', () => {
    // `landingState` is initialised `{ build: 'dev', dismissed: false }`, and a
    // `phoenix-host --world …` never pushes a landing to correct it:
    // `feed_landing_panel` requires a `LobbyScenarioCatalog` and
    // src/native_host/app.rs inserts one only for a world-less host (the Rust
    // test `a_host_with_no_catalogue_never_publishes_a_landing_either` pins
    // exactly that). `feed_lobby_state` has no such gate, so the lobby pushes
    // arrive regardless — and `!landingState.dismissed` read as “the landing is
    // up” would answer 'hide' on every one of them, taking the QR, the URL and
    // the typed code away from the whole run and re-hiding the panel behind the
    // operator's own toggle (issue #1329).
    expect(LINK_JS).toContain('let landingPushed = false;');
    expect(LINK_JS).toMatch(
      /function landingUp\(\)\s*\{\s*return landingPushed && !landingState\.dismissed;\s*\}/,
    );
    expect(LINK_JS).not.toContain('landingUp: !landingState.dismissed');
    // Both call sites — the lobby payload's, and the landing push's own re-ask.
    expect(LINK_JS).toMatch(/hostLobbyViewModel\([\s\S]{0,120}?landingUp: landingUp\(\)/);
    expect(LINK_JS).toContain('joinPanelAction(prevPhase, landingUp())');
    // And a landing exists from the first push, whatever that push says.
    const pushed = linkFunctionBody('window.__phoenixHostLobby.renderLanding = function (json) {');
    expect(pushed).toContain('landingPushed = true;');
    expect(pushed.indexOf('landingPushed = true'))
      .toBeLessThan(pushed.indexOf('applyQrPhase('));
  });

  it('refuses every toggle while the landing is in front of the panel', () => {
    // This surface has three ways in — the settings row's verb, the control's
    // click, its keyboard parity — plus the presses a phone's ToggleQrCode
    // arrives as on renderJoin, which is why the VERB is guarded and not the
    // button: hiding #host-lobby-qr-toggle would leave the phone's path open.
    // One press over the landing would otherwise stick, since the lobby push is
    // deduped in Rust and the landing push only fires when the landing moves.
    expect(LINK_JS.match(/toggleQr\(document\)/g)).toHaveLength(1);
    const guard = linkFunctionBody('function requestToggleQr() {');
    expect(guard).toContain('if (joinPanelSuppressed(landingUp())) return;');
    expect(guard).toContain('toggleQr(document);');
    for (const site of [
      'toggle_qr: () => requestToggleQr(),',
      "qrToggle.addEventListener('click', () => requestToggleQr());",
      'for (let i = 0; i < qrToggles; i += 1) requestToggleQr();',
    ]) {
      expect(LINK_JS).toContain(site);
    }
  });
});
