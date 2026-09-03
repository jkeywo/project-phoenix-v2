/**
 * tests/client/qr-encoder.test.js — the vendored QR encoder (issue #1329).
 *
 * The join QR is the one thing a crew cannot start without, and until this
 * issue it hung off a CDN `<script>`: no internet in the room, no code on the
 * viewscreen, and on the NATIVE host — which composites the same lobby out of
 * its own bundle — no code at all even with internet, because an embedded view
 * on a LAN-only bridge machine is exactly the case the CDN cannot serve.
 *
 * So the encoder is vendored (`gui/vendor/qrcode.js`) and served by whichever
 * process serves the page. This file pins the three things that can quietly
 * stop being true:
 *
 *   1. the vendored bytes are the upstream artefact the README claims, not
 *      something edited in place;
 *   2. the library still works when loaded from disk with no network at all;
 *   3. `server.html` loads THAT file, and no CDN.
 *
 * The native lobby document's half of (3) is asserted in Rust, where that
 * document is assembled — `native_host::host_lobby::document`'s tests.
 */
import { describe, it, expect } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
const VENDOR = path.join(ROOT, 'gui', 'vendor');
const ENCODER = path.join(VENDOR, 'qrcode.js');

/**
 * Load the vendored bundle the way a browser does — as a script that assigns a
 * global — but in Node, with no DOM and no network.
 *
 * That it evaluates at all under those conditions is half the point of the
 * test: this file has to work inside an Ultralight view on a bridge machine
 * with the network cable out.
 */
function loadEncoder() {
  const src = fs.readFileSync(ENCODER, 'utf8');
  return new Function(`${src}\nreturn QRCode;`)();
}

describe('gui/vendor/qrcode.js — the vendored encoder', () => {
  it('is the upstream artefact gui/vendor/README.md names, unedited', () => {
    // A local "small fix" to vendored bytes is how a vendored dependency
    // becomes an unmaintained fork nobody remembers forking. The hash is
    // `qrcode@1.5.1`'s own `build/qrcode.js`, from the npm registry tarball —
    // and byte-identical to what cdn.jsdelivr.net was serving for
    // `npm/qrcode/build/qrcode.min.js`, which is what this replaced.
    const sha = crypto.createHash('sha256').update(fs.readFileSync(ENCODER)).digest('hex');
    expect(sha).toBe('ba588dfaf738bf8980e5da3b680ab1ce3f205af7577454c16f9c0506fe744df4');

    const readme = fs.readFileSync(path.join(VENDOR, 'README.md'), 'utf8');
    expect(readme).toContain(sha);
    expect(readme).toContain('1.5.1');
  });

  it('carries the licence its terms require to travel with the copy', () => {
    const licence = fs.readFileSync(path.join(VENDOR, 'qrcode.LICENSE.txt'), 'utf8');
    expect(licence).toContain('MIT License');
    expect(licence).toContain('Ryan Day');
    expect(licence).toContain('shall be included in all copies');
  });

  it('loads from disk with no network and exposes the API both surfaces call', () => {
    const QRCode = loadEncoder();
    // `toCanvas` is what the host page and the native lobby document draw
    // with; `create` is the pure half this file asserts on below.
    expect(typeof QRCode.toCanvas).toBe('function');
    expect(typeof QRCode.create).toBe('function');
  });

  it('encodes a known join URL to a stable code', () => {
    // The output is pinned, not merely non-empty: a QR that encodes SOMETHING
    // looks identical to a working one on a viewscreen across a room, and the
    // failure only shows up when a phone will not scan it. A refreshed vendor
    // copy that changes these numbers is a change to what the room scans, and
    // should be looked at rather than re-pinned reflexively.
    const QRCode = loadEncoder();
    const url = 'http://192.168.1.5:8080/client/index.html#TEST-CODE';
    const code = QRCode.create(url, { errorCorrectionLevel: 'M' });

    expect(code.version).toBe(4);
    expect(code.modules.size).toBe(33);

    const bits = Array.from(code.modules.data, (m) => (m ? '1' : '0')).join('');
    expect(crypto.createHash('sha256').update(bits).digest('hex'))
      .toBe('c7c0c508753e65f935e2d869942783438e869896c61642549046de8b7079843a');
    // The three finder patterns' top-left corner, so a failure reads as "the
    // code changed shape" rather than only as "a hash moved".
    expect(bits.slice(0, 7)).toBe('1111111');
  });
});

describe('server.html loads the local copy', () => {
  const page = fs.readFileSync(path.join(ROOT, 'server.html'), 'utf8');

  it('points its encoder <script> at gui/vendor/qrcode.js', () => {
    expect(page).toContain('<script src="gui/vendor/qrcode.js"></script>');
  });

  it('makes no CDN request for it, from any page', () => {
    // The whole bundle, not just this tag: a second CDN reference anywhere
    // would restore the dependency this issue removed.
    // The URL, not the word: the tag's replacement comment says what used to
    // be there and why it is not any more, and that sentence is worth keeping.
    for (const file of ['server.html', 'client.html']) {
      expect(fs.readFileSync(path.join(ROOT, file), 'utf8')).not.toContain('cdn.jsdelivr.net');
    }
    // Google Fonts is a `<link>`, deliberately kept and deliberately not a
    // script: it degrades to the fallback face rather than to no QR.
    expect(/<script[^>]+src=["']https?:/i.test(page)).toBe(false);
  });
});
