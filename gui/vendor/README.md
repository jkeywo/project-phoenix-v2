# `gui/vendor/` — third-party code served by the host's own delivery server

Everything in this directory is **somebody else's source, carried verbatim**.
Nothing here is edited: a local change would make the provenance line below a
lie, and the hash the drift test pins would stop matching. If one of these needs
to behave differently, wrap it in a `gui/` module of ours and leave the vendored
bytes alone.

## Why vendor at all

A bridge machine is not assumed to have internet (issue #1329, PRD #1324). The
native host serves its own bundle to the phones in the room and composites its
own lobby surface out of that same bundle, so a `<script src="https://cdn…">`
in the host page is a dependency on a network the room may not have — and it is
the *join* QR, the one thing a crew cannot start without, that was hanging off
it. Same-origin, served by the process that is already serving everything else.

## `qrcode.js`

| | |
|---|---|
| upstream | [`node-qrcode`](https://github.com/soldair/node-qrcode) |
| package | `qrcode`, version **1.5.1**, file `build/qrcode.js` |
| licence | MIT — `qrcode.LICENSE.txt`, copied from the package's own `license` |
| sha256 | `ba588dfaf738bf8980e5da3b680ab1ce3f205af7577454c16f9c0506fe744df4` |

It is the **pre-minified browser build** the package publishes (hence a `.js`
name on minified bytes — the upstream file name, kept), an IIFE that assigns the
global `QRCode` with `create`, `toCanvas`, `toDataURL` and `toString`.

It is also, byte for byte, what `server.html` was fetching from
`https://cdn.jsdelivr.net/npm/qrcode/build/qrcode.min.js` before #1329 — that
URL serves this exact file (jsdelivr reports it as
`/npm/qrcode@1.5.1/build/qrcode.js`, minification skipped, because 1.5.2 dropped
`build/` from the published tarball). Verified by `cmp` against the registry
tarball at vendoring time, which is why the hash above is checkable from two
independent sources.

**Version 1.5.1 is deliberate, not stale.** The versions after it publish no
browser bundle at all, so "upgrade to latest" here means "add a bundler step to
this repository". Read the changelog before deciding that is worth it.

### Refreshing it

```bash
npm pack qrcode@<version>          # from the registry, not the CDN
tar -xzf qrcode-<version>.tgz
cp package/build/qrcode.js  gui/vendor/qrcode.js
cp package/license          gui/vendor/qrcode.LICENSE.txt
```

Then update the version and hash in the table above and in
`tests/client/qr-encoder.test.js`, which pins both.
