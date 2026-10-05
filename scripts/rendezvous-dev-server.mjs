#!/usr/bin/env node
// scripts/rendezvous-dev-server.mjs — the rendezvous service on localhost,
// with no Cloudflare account and no wrangler (issue #1113).
//
// It runs the REAL `worker-rendezvous/src/registry.js` — the same module the
// Worker bundles and the same one the contract tests drive — over real
// WebSocket connections. What is local is the socket termination and the
// origin gate, which is `src/index.js`'s job; everything the service DECIDES
// comes from the shipped module.
//
// ## Why this exists
//
// Three of the things #1113 built cannot be proved by any test that stops at a
// function boundary: that `tungstenite` and a Cloudflare-shaped WebSocket agree
// on framing, that the native host's Rust frame vocabulary and the JavaScript
// one really interoperate, and that a browser page can join a native host. All
// three need a socket. Deploying to Cloudflare to get one is a credentialed
// step this repository cannot take (docs/delivery-checklist.md), and
// `wrangler dev` is a large install that no other part of this repo needs.
//
// So: `node:http` plus `node:crypto`, no dependencies, in the same
// dependency-free spirit as `scripts/check-deploy-headers.mjs`. It speaks
// enough of RFC 6455 for two Phoenix clients to talk — text frames, close,
// ping/pong — and deliberately not a byte more.
//
// ## What it is NOT
//
//   * NOT a deployment. No TLS, no origin allowlist worth the name, no bounds
//     beyond the registry's own. Bind it to loopback and nothing else.
//   * NOT a substitute for the deployed service in acceptance. The whole point
//     of docs/acceptance/1113-networks.md is what real networks do to real
//     traffic, and a loopback socket meets none of it.
//   * NOT covered by CI. Nothing in .github runs this; it is a local aid.
//
// ## Running it
//
//   node scripts/rendezvous-dev-server.mjs [--port 8788]
//     [--delay-ms 0..2000] [--loss-percent 0..100] [--seed 0..4294967295]
//     [--block-rtc-offers]
//
// Then point either end at it:
//
//   client/index.html?rendezvous=http://localhost:8788#<code>   (loopback only —
//       gui/rendezvous-transport.js refuses a non-loopback override)
//   phoenix-host --world … --rendezvous http://127.0.0.1:8788 \
//       --origin http://localhost:3000
//
// GET /__impairment reports this process's configured profile and actual frame
// counts. Delay applies to outbound WebSocket game relay frames of both classes.
// Loss applies only to snapshot-class relay frames; reliable frames are ordered
// and never sampled for loss. --block-rtc-offers suppresses relayed SDP offers
// while leaving code lookup, join and the WebSocket game relay intact. These
// are application frames, not IP packets; "written" means handed to the local
// socket, not acknowledged by a remote peer. The seed fixes loss decisions for
// the same frame arrival order. Direct WebRTC DataChannel traffic bypasses this
// adapter and is unimpaired. No profile simulates mobile radio behavior.

import http from 'node:http';
import crypto from 'node:crypto';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { performance } from 'node:perf_hooks';

import { createRegistry, ROLE_HOST, ROLE_CLIENT } from '../worker-rendezvous/src/registry.js';
import { setJoinCodeData } from '../packages/transport/src/join-code.js';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const codesArg = process.argv.indexOf('--codes');
const codesPath = codesArg >= 0 ? process.argv[codesArg + 1] : path.join(root, 'assets/join/join-codes.json');
if (!codesPath) throw new Error('--codes requires a table path');
const DATA = JSON.parse(readFileSync(codesPath, 'utf8'));
setJoinCodeData(DATA);

/** The GUID RFC 6455 mixes into the accept key. Not a secret; it is the spec. */
const WS_GUID = '258EAFA5-E914-47DA-95CA-C5AB0DC85B11';

// These knobs affect only this local socket adapter. The registry and the
// deployed Worker retain their ordinary delivery contract.
const MAX_PENDING_FRAMES = 4096;
function numberFlag(args, name, fallback, max) {
  const at = args.indexOf(name);
  if (at < 0) return fallback;
  const value = args[at + 1];
  if (!/^(0|[1-9]\d*)$/.test(value || '') || Number(value) > max) {
    throw new Error(`${name} must be an integer from 0 to ${max}`);
  }
  return Number(value);
}
const argv = process.argv.slice(2);
const PORT = numberFlag(argv, '--port', 8788, 65535);
if (PORT === 0) throw new Error('--port must be from 1 to 65535');
const profile = Object.freeze({
  delay_ms: numberFlag(argv, '--delay-ms', 0, 2000),
  loss_percent: numberFlag(argv, '--loss-percent', 0, 100),
  seed: numberFlag(argv, '--seed', 1, 0xffffffff),
  block_rtc_offers: argv.includes('--block-rtc-offers'),
});
const knownFlags = new Set(['--codes', '--port', '--delay-ms', '--loss-percent', '--seed', '--block-rtc-offers']);
for (let i = 0; i < argv.length; i += 1) {
  if (!knownFlags.has(argv[i])) throw new Error(`unknown argument: ${argv[i]}`);
  if (argv[i] !== '--block-rtc-offers') i += 1;
}

const registry = createRegistry({ data: DATA });
/** @type {Map<string, {socket: import('node:net').Socket, pending: object[], timer: ReturnType<typeof setTimeout>|null, lastDue: number}>} */
const sockets = new Map();
const delayStats = () => ({ count: 0, min: null, max: null, total: 0 });
const counters = {
  relay_reliable_seen: 0, relay_reliable_written: 0, relay_reliable_cancelled: 0,
  relay_snapshot_seen: 0, relay_snapshot_written: 0,
  relay_snapshot_dropped: 0, relay_snapshot_cancelled: 0,
  relay_frames_delayed: 0, signal_offers_dropped: 0,
  queue_overflow_closes: 0, pending: 0, peak_pending: 0,
  observed_write_delay_ms: { reliable: delayStats(), snapshot: delayStats() },
};
// LCG with a fully specified 32-bit sequence, so a given seed and arrival
// order produce the same snapshot-drop decisions across Node versions.
let randomState = profile.seed;
function drawLoss() {
  randomState = (Math.imul(randomState, 1664525) + 1013904223) >>> 0;
  return randomState / 0x100000000 < profile.loss_percent / 100;
}

function writeFrame(entry, frame, enqueuedAt = null) {
  sendText(entry.socket, JSON.stringify(frame));
  if (frame.type === 'relay' && frame.class === 'reliable') counters.relay_reliable_written += 1;
  if (frame.type === 'relay' && frame.class === 'snapshot') counters.relay_snapshot_written += 1;
  if (enqueuedAt !== null && frame.type === 'relay') {
    const observed = performance.now() - enqueuedAt;
    const stats = counters.observed_write_delay_ms[frame.class];
    stats.count += 1;
    stats.total += observed;
    stats.min = stats.min === null ? observed : Math.min(stats.min, observed);
    stats.max = stats.max === null ? observed : Math.max(stats.max, observed);
  }
}

function pump(connId) {
  const entry = sockets.get(connId);
  if (!entry) return;
  entry.timer = null;
  const now = performance.now();
  while (entry.pending.length && entry.pending[0].due <= now) {
    const { frame, enqueuedAt } = entry.pending.shift();
    counters.pending -= 1;
    writeFrame(entry, frame, enqueuedAt);
  }
  if (entry.pending.length) {
    entry.timer = setTimeout(() => pump(connId), Math.max(0, entry.pending[0].due - performance.now()));
  }
}

function queueFrame(connId, entry, frame) {
  if (counters.pending >= MAX_PENDING_FRAMES) {
    if (frame.type === 'relay' && frame.class === 'snapshot') {
      counters.relay_snapshot_dropped += 1;
    } else {
      // Closing is the honest result when the local adapter cannot retain a
      // reliable frame. It must never silently turn reliable into lossy.
      counters.queue_overflow_closes += 1;
      if (frame.type === 'relay' && frame.class === 'reliable') {
        counters.relay_reliable_cancelled += 1;
      }
      endSocket(connId, 1013, 'impairment queue full');
    }
    return;
  }
  // A control frame behind delayed game traffic must not overtake it on this
  // same socket. It takes no extra delay of its own.
  const isRelay = frame.type === 'relay';
  const enqueuedAt = performance.now();
  const due = Math.max(enqueuedAt + (isRelay ? profile.delay_ms : 0), entry.lastDue);
  entry.lastDue = due;
  entry.pending.push({ frame, due, enqueuedAt });
  counters.pending += 1;
  counters.peak_pending = Math.max(counters.peak_pending, counters.pending);
  if (isRelay) counters.relay_frames_delayed += 1;
  if (!entry.timer) entry.timer = setTimeout(() => pump(connId), Math.max(0, due - performance.now()));
}

function dispatch(frames) {
  for (const { to, frame, close } of frames) {
    const relayClass = frame.type === 'relay' ? frame.class : null;
    if (relayClass === 'reliable') counters.relay_reliable_seen += 1;
    if (relayClass === 'snapshot') counters.relay_snapshot_seen += 1;
    if (profile.block_rtc_offers && frame.type === 'signal'
        && frame.payload?.sdp?.type === 'offer') {
      counters.signal_offers_dropped += 1;
      continue;
    }
    const entry = sockets.get(to);
    if (!entry) {
      if (relayClass === 'snapshot') counters.relay_snapshot_dropped += 1;
      if (relayClass === 'reliable') counters.relay_reliable_cancelled += 1;
      continue;
    }
    if (relayClass === 'snapshot' && profile.loss_percent && drawLoss()) {
      counters.relay_snapshot_dropped += 1;
      continue;
    }
    if (!close && ((relayClass && profile.delay_ms) || entry.pending.length)) {
      queueFrame(to, entry, frame);
    } else {
      writeFrame(entry, frame);
    }
    if (close) endSocket(to, 1008, frame.reason || 'refused');
  }
}

// ── The smallest RFC 6455 that two Phoenix clients need ─────────────────────
//
// Text frames out (never masked — a server must not mask), text frames in
// (always masked — a client must), plus close and ping. Fragmentation is not
// implemented: neither `tungstenite` nor a browser fragments a JSON frame this
// size, and a half-implemented reassembler would be a worse bug than an honest
// refusal.

function sendText(socket, text) {
  const payload = Buffer.from(text, 'utf8');
  const header = frameHeader(0x1, payload.length);
  socket.write(Buffer.concat([header, payload]));
}

function frameHeader(opcode, length) {
  if (length < 126) return Buffer.from([0x80 | opcode, length]);
  if (length < 65536) {
    const b = Buffer.alloc(4);
    b[0] = 0x80 | opcode;
    b[1] = 126;
    b.writeUInt16BE(length, 2);
    return b;
  }
  const b = Buffer.alloc(10);
  b[0] = 0x80 | opcode;
  b[1] = 127;
  b.writeBigUInt64BE(BigInt(length), 2);
  return b;
}

function removeConnection(connId) {
  const entry = sockets.get(connId);
  if (!entry) return null;
  sockets.delete(connId);
  if (entry.timer) clearTimeout(entry.timer);
  for (const { frame } of entry.pending) {
    if (frame.type === 'relay' && frame.class === 'snapshot') {
      counters.relay_snapshot_cancelled += 1;
    }
    if (frame.type === 'relay' && frame.class === 'reliable') {
      counters.relay_reliable_cancelled += 1;
    }
    counters.pending -= 1;
  }
  return entry;
}

function endSocket(connId, code = 1000, reason = '') {
  const entry = removeConnection(connId);
  if (!entry) return;
  const body = Buffer.concat([
    Buffer.from([(code >> 8) & 0xff, code & 0xff]),
    Buffer.from(String(reason).slice(0, 120), 'utf8'),
  ]);
  try {
    entry.socket.write(Buffer.concat([frameHeader(0x8, body.length), body]));
    entry.socket.end();
  } catch { /* already gone */ }
  dispatch(registry.disconnect(connId));
}

/** Pull as many whole frames as `buf` holds; returns the unconsumed remainder. */
function readFrames(buf, onText, onClose, onPing) {
  let offset = 0;
  for (;;) {
    if (buf.length - offset < 2) break;
    const first = buf[offset];
    const second = buf[offset + 1];
    const opcode = first & 0x0f;
    const masked = (second & 0x80) !== 0;
    let length = second & 0x7f;
    let cursor = offset + 2;
    if (length === 126) {
      if (buf.length < cursor + 2) break;
      length = buf.readUInt16BE(cursor);
      cursor += 2;
    } else if (length === 127) {
      if (buf.length < cursor + 8) break;
      length = Number(buf.readBigUInt64BE(cursor));
      cursor += 8;
    }
    let mask = null;
    if (masked) {
      if (buf.length < cursor + 4) break;
      mask = buf.subarray(cursor, cursor + 4);
      cursor += 4;
    }
    if (buf.length < cursor + length) break;
    const payload = Buffer.from(buf.subarray(cursor, cursor + length));
    if (mask) for (let i = 0; i < payload.length; i += 1) payload[i] ^= mask[i % 4];
    cursor += length;
    offset = cursor;

    if (opcode === 0x1) onText(payload.toString('utf8'));
    else if (opcode === 0x8) { onClose(); return buf.subarray(offset); }
    else if (opcode === 0x9) onPing(payload);
  }
  return buf.subarray(offset);
}

const server = http.createServer((req, res) => {
  // The one HTTP endpoint the deploy check and the checklist recipe use.
  if (req.url && req.url.startsWith('/v1/health')) {
    res.writeHead(200, { 'Content-Type': 'application/json', 'Cache-Control': 'no-store' });
    res.end(JSON.stringify({
      ok: true,
      service: 'phoenix-rendezvous',
      protocol: registry.protocol,
      format_version: DATA.format_version,
      origin: req.headers.origin || null,
      // A dev server allows everything; it is loopback-only and says so.
      origin_allowed: true,
      dev: true,
    }));
    return;
  }
  if (req.url === '/__impairment') {
    res.writeHead(200, { 'Content-Type': 'application/json', 'Cache-Control': 'no-store' });
    res.end(JSON.stringify({
      profile,
      scope: 'local WebSocket relay frames only; direct DataChannel traffic bypasses this adapter; loss applies only to snapshot class',
      counters,
      max_pending_frames: MAX_PENDING_FRAMES,
    }));
    return;
  }
  res.writeHead(404, { 'Content-Type': 'application/json' });
  res.end(JSON.stringify({ error: 'not-found' }));
});

server.on('upgrade', (req, socket, head) => {
  const key = req.headers['sec-websocket-key'];
  if (!key) {
    socket.destroy();
    return;
  }
  const accept = crypto.createHash('sha1').update(key + WS_GUID).digest('base64');
  socket.write(
    'HTTP/1.1 101 Switching Protocols\r\n'
    + 'Upgrade: websocket\r\n'
    + 'Connection: Upgrade\r\n'
    + `Sec-WebSocket-Accept: ${accept}\r\n\r\n`,
  );
  socket.setNoDelay(true);

  const connId = crypto.randomUUID();
  const role = (req.url || '').endsWith('/v1/host') ? ROLE_HOST : ROLE_CLIENT;
  sockets.set(connId, { socket, pending: [], timer: null, lastDue: 0 });

  let buffer = head && head.length ? Buffer.from(head) : Buffer.alloc(0);
  socket.on('data', (chunk) => {
    buffer = Buffer.concat([buffer, chunk]);
    buffer = readFrames(
      buffer,
      (text) => {
        let frame = null;
        try { frame = JSON.parse(text); } catch { frame = null; }
        dispatch(registry.receive(connId, frame));
      },
      () => endSocket(connId),
      (payload) => socket.write(Buffer.concat([frameHeader(0xa, payload.length), payload])),
    );
  });
  const gone = () => {
    if (!removeConnection(connId)) return;
    dispatch(registry.disconnect(connId));
  };
  socket.on('close', gone);
  socket.on('error', gone);

  dispatch(registry.connect(connId, role));
  console.log(`[rendezvous-dev] ${role} socket ${connId.slice(0, 8)}…`);
});

server.listen(PORT, '127.0.0.1', () => {
  console.log(`[rendezvous-dev] listening on http://127.0.0.1:${PORT}`);
  console.log(`[rendezvous-dev] impairment ${JSON.stringify(profile)}`);
  console.log('[rendezvous-dev] loopback only — this is a local aid, never a deployment');
});
