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

  it('shows the five letters a guest types instead of scanning', () => {
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
  // `page_base` comes from `native_host::host_lobby::join`, which pins the same
  // literal in `an_invitation_carries_the_letters_the_code_and_where_to_go` —
  // and the URL is built by the very function the browser host builds its own
  // from, so "the native QR points somewhere else" is not a shape this can take.
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

  it('keeps no second notion of whether the panel is on screen', () => {
    // The `qrVisible` flag four handlers used to keep in step. The state is
    // #overlay, and gui/host-qr.js is the only thing that reads or writes it.
    expect(SERVER_HTML).not.toMatch(/^\s*let qrVisible/m);
  });
});
