// The client bundle's delivery stamp (issue #1111).
//
// `src/delivery/stamp.rs` pins a Phoenix host to three numbers: the wire
// protocol revision and the served content set's id/epoch. A native host reads
// a caller's copy off the `x-phoenix-client-stamp: <protocol>/<id>/<epoch>`
// header. The browser client has no WASM to compile a version into, so the
// same field is written into `<meta name="phoenix-client-stamp">` at build
// time and presented over the join handshake instead.
//
// Pure function + thin caller, the house split: this module reads files and
// returns a string, `build-client.mjs` writes it into the page, and
// `tests/client/client-stamp.test.js` checks the parsing against the real
// repository sources — so a `PROTOCOL_VERSION` bump or a `[content]` epoch bump
// cannot silently stop reaching the client.

import { readFile } from 'node:fs/promises';
import path from 'node:path';

/** `pub const PROTOCOL_VERSION: u32 = N;` → N. */
export function protocolVersionFrom(messagesRs) {
  const m = /pub\s+const\s+PROTOCOL_VERSION\s*:\s*u32\s*=\s*(\d+)\s*;/.exec(messagesRs);
  return m ? Number(m[1]) : null;
}

/** The manifest's `[content]` identity, mirroring `parse_content_identity`. */
export function contentIdentityFrom(manifestToml) {
  const lines = String(manifestToml).split(/\r?\n/);
  const start = lines.findIndex((l) => l.trim() === '[content]');
  if (start < 0) return null;
  let id = null;
  let epoch = null;
  for (const line of lines.slice(start + 1)) {
    if (line.trim().startsWith('[')) break;
    const idMatch = /^\s*id\s*=\s*"([^"]*)"/.exec(line);
    if (idMatch) id = idMatch[1];
    const epochMatch = /^\s*epoch\s*=\s*(-?\d+)/.exec(line);
    if (epochMatch) epoch = Number(epochMatch[1]);
  }
  return id != null && epoch != null ? { id, epoch } : null;
}

/** Assemble the `<protocol>/<content_id>/<content_epoch>` field. */
export function stampField(protocol, content) {
  if (protocol == null || !content) return '';
  return `${protocol}/${content.id}/${content.epoch}`;
}

/**
 * Read the field out of the repository at `root`.
 *
 * Returns `''` when either source cannot be read. An empty stamp means
 * "unstamped", which since issue #1112 a host REFUSES — so a build that lands
 * here has produced a client no host will admit. It still does not fail the
 * build: the failure is loud and immediate at the first join attempt, with a
 * named reason (`client-stamp-missing`), whereas throwing here would let an
 * unrelated refactor of messages.rs break the client build outright. Both
 * sources are committed files that always exist in a real checkout.
 */
export async function clientStampField(root) {
  try {
    const [messages, manifest] = await Promise.all([
      readFile(path.join(root, 'src/core/messages.rs'), 'utf8'),
      readFile(path.join(root, 'assets/scenarios.toml'), 'utf8'),
    ]);
    return stampField(protocolVersionFrom(messages), contentIdentityFrom(manifest));
  } catch {
    return '';
  }
}
