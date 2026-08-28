// The deploy contract for the two workers (issue #1113).
//
// scripts/rendezvous-checks.mjs turns docs/delivery-checklist.md's §3 and §3a
// curl recipes into judgements a machine makes. These fixtures are the failure
// modes those recipes exist for — above all the 2026-08 one, where the repo
// file was correct, the deployed worker was not, and nothing said so until a
// room full of phones could not connect.

import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  checkRendezvousHealth,
  checkOriginIsRefused,
  checkUpgradeRequired,
  checkTurnWorker,
  normaliseHeaders,
  summarise,
} from '../../scripts/rendezvous-checks.mjs';
import { RENDEZVOUS_PROTOCOL } from '../../gui/rendezvous-protocol.js';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const TABLE = JSON.parse(readFileSync(path.join(root, 'assets/join/join-codes.json'), 'utf8'));

const ORIGIN = 'https://pp-dev.kiwigamedesign.co.uk';

/** A healthy rendezvous /v1/health probe. */
const health = (over = {}) => ({
  url: 'https://phoenix-rendezvous.project-phoenix.workers.dev/v1/health',
  origin: ORIGIN,
  status: 200,
  headers: { 'content-type': 'application/json', 'cache-control': 'no-store' },
  formatVersion: TABLE.format_version,
  ...over,
  body: {
    ok: true,
    service: 'phoenix-rendezvous',
    protocol: RENDEZVOUS_PROTOCOL,
    format_version: TABLE.format_version,
    origin: ORIGIN,
    origin_allowed: true,
    ...(over.body || {}),
  },
});

/** A healthy TURN credential probe. */
const turn = (over = {}) => ({
  url: 'https://phoenix-turn-credentials.project-phoenix.workers.dev',
  origin: ORIGIN,
  status: 200,
  headers: { 'access-control-allow-origin': ORIGIN, 'cache-control': 'no-store' },
  body: [
    { urls: 'stun:stun.example:3478' },
    { urls: 'turn:relay.example:443', username: 'u', credential: 'c' },
  ],
  ...over,
});

const ids = (findings) => findings.map((f) => f.id);

describe('the rendezvous worker', () => {
  it('passes a healthy deployment', () => {
    expect(checkRendezvousHealth(health())).toEqual([]);
  });

  it('catches the 2026-08 failure: a stale allowlist that refuses our own origin', () => {
    // The whole reason /v1/health carries this field. For TURN a stale
    // allowlist merely stripped relay; here it means nobody can join at all,
    // and the page shows no code and no reason.
    const findings = checkRendezvousHealth(health({ body: { origin_allowed: false } }));
    expect(ids(findings)).toContain('origin-not-allowed');
    expect(findings[0].level).toBe('error');
    // The remedy has to be IN the finding: the value being wrong in the repo is
    // not the failure, deploying it is.
    expect(findings[0].detail).toContain('wrangler deploy');
  });

  it('refuses a plain-http service, which a secure page cannot open a socket to', () => {
    const findings = checkRendezvousHealth(health({ url: 'http://rendezvous.example/v1/health' }));
    expect(ids(findings)).toContain('tls');
  });

  it('catches a service deployed from another checkout', () => {
    // Both ends hard-refuse a foreign `v`, so this is a total outage rather
    // than a degradation — and it looks exactly like "the service is down".
    expect(ids(checkRendezvousHealth(health({ body: { protocol: RENDEZVOUS_PROTOCOL + 1 } }))))
      .toContain('protocol');
    expect(ids(checkRendezvousHealth(health({ body: { format_version: 99 } }))))
      .toContain('format-version');
  });

  it('reports an unreachable or unhealthy service before checking anything else', () => {
    expect(ids(checkRendezvousHealth(health({ status: 503 })))).toEqual(['health-status']);
    expect(ids(checkRendezvousHealth(health({ body: { ok: false } })))).toEqual(['health-body']);
  });

  it('warns when liveness is cacheable', () => {
    expect(ids(checkRendezvousHealth(health({ headers: {} })))).toContain('health-cache');
  });

  it('notices an allowlist that says yes to everybody', () => {
    // `ALLOWED_ORIGIN = "*"` is a plausible way to "fix" a CORS problem under
    // time pressure, and it passes every other check in this file while turning
    // an unauthenticated public endpoint into one anybody can register against.
    const control = { origin: 'https://not-a-phoenix-origin.invalid', status: 200, body: { origin_allowed: true } };
    expect(ids(checkOriginIsRefused(control))).toEqual(['origin-allowlist-open']);
    expect(checkOriginIsRefused({ ...control, body: { origin_allowed: false } })).toEqual([]);
  });

  it('expects a socket endpoint to demand an upgrade', () => {
    expect(checkUpgradeRequired({ status: 426, path: '/v1/host' })).toEqual([]);
    // A 200 here would mean the endpoint is answering something that is not a
    // socket, which is a worker serving the wrong script.
    expect(ids(checkUpgradeRequired({ status: 200, path: '/v1/host' }))).toEqual(['upgrade']);
  });
});

describe('the TURN credential worker', () => {
  it('passes a healthy deployment', () => {
    expect(checkTurnWorker(turn())).toEqual([]);
  });

  it('catches the 2026-08 failure itself: an echoed origin that is not ours', () => {
    // The deployed worker still carried a pre-custom-domain allowlist, so it
    // echoed the FIRST entry instead of the origin asked. Every browser fetch
    // was CORS-blocked and every player silently dropped to the free relay.
    const findings = checkTurnWorker(turn({
      headers: { 'access-control-allow-origin': 'https://jkeywo.github.io', 'cache-control': 'no-store' },
    }));
    expect(ids(findings)).toContain('turn-cors');
    expect(findings[0].detail).toContain('silently');
  });

  it('reports a worker whose every credential source failed', () => {
    expect(ids(checkTurnWorker(turn({ status: 502 })))).toEqual(['turn-sources']);
  });

  it('warns when one source is down and the other is carrying it', () => {
    // 200 with the header is the "half-broken but working" state §3 asks an
    // operator to notice before it becomes the only source.
    const findings = checkTurnWorker(turn({
      headers: {
        'access-control-allow-origin': ORIGIN,
        'cache-control': 'no-store',
        'x-turn-source-errors': 'metered: 401',
      },
    }));
    expect(ids(findings)).toEqual(['turn-source-degraded']);
    expect(findings[0].level).toBe('warn');
  });

  it('catches a 200 that carries no relay at all', () => {
    // The most misleading answer available: everything looks fine and every
    // player on a mobile network fails to connect.
    expect(ids(checkTurnWorker(turn({ body: [{ urls: 'stun:stun.example:3478' }] }))))
      .toContain('turn-no-relay');
    expect(ids(checkTurnWorker(turn({ body: [] })))).toEqual(['turn-empty']);
  });
});

describe('the verdict', () => {
  it('fails on an error and survives a warning', () => {
    expect(summarise([{ level: 'warn', id: 'w' }]).ok).toBe(true);
    expect(summarise([{ level: 'error', id: 'e' }]).ok).toBe(false);
  });

  it('reads a header name whatever its casing', () => {
    // `fetch`'s Headers are case-insensitive; a plain object from a fixture or
    // a curl transcript is not.
    expect(normaliseHeaders({ 'Cache-Control': 'no-store' })['cache-control']).toBe('no-store');
    expect(normaliseHeaders(new Map([['X-Turn-Source-Errors', 'a']]))['x-turn-source-errors']).toBe('a');
    expect(normaliseHeaders(null)).toEqual({});
  });
});
