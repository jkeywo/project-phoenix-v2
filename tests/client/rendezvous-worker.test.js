// The rendezvous Worker's request gate (issue #1111).
//
// worker-rendezvous/src/index.js decides nothing about the protocol — that is
// src/registry.js, covered by rendezvous-registry.test.js — but it does decide
// WHO gets a socket at all, and that gate is a real refusal rather than a
// browser-enforced CORS header. These tests drive the exported fetch handler
// directly with Node's own Request/Response, so they need no wrangler and no
// miniflare; the Durable Object binding is a stub, because every case here is
// answered before the request would reach it.

import { describe, it, expect } from 'vitest';

import worker from '../../worker-rendezvous/src/index.js';

const ALLOWED = 'https://pp-dev.kiwigamedesign.co.uk';
const env = () => ({
  ALLOWED_ORIGIN: `${ALLOWED},http://localhost:3000`,
  // Reaching this means the gate let the request through.
  RENDEZVOUS: {
    idFromName: () => 'id',
    // Node's Response cannot be constructed with 101, so the stub answers
    // with a marker the assertions below look for instead.
    get: () => ({ fetch: async () => new Response('upgraded-by-the-durable-object') }),
  },
});

const upgrade = (path, origin) =>
  new Request(`https://phoenix-rendezvous.workers.dev${path}`, {
    headers: {
      Upgrade: 'websocket',
      ...(origin ? { Origin: origin } : {}),
    },
  });

describe('origin gate', () => {
  for (const path of ['/v1/host', '/v1/join']) {
    it(`refuses ${path} outright when the request carries no Origin`, async () => {
      // The pages live on pp-dev/pp-demo and this service on *.workers.dev, so
      // every legitimate browser upgrade is cross-origin and always carries an
      // Origin. Exempting requests without one exempts exactly the scripted
      // non-browser caller this 403 exists to stop.
      const res = await worker.fetch(upgrade(path), env());
      expect(res.status).toBe(403);
      expect((await res.json()).error).toBe('origin-not-allowed');
    });

    it(`refuses ${path} from an origin that is not on the list`, async () => {
      const res = await worker.fetch(upgrade(path, 'https://evil.test'), env());
      expect(res.status).toBe(403);
    });

    it(`upgrades ${path} from an allow-listed origin`, async () => {
      const res = await worker.fetch(upgrade(path, ALLOWED), env());
      expect(await res.text()).toBe('upgraded-by-the-durable-object');
    });
  }

  it('still answers /v1/health to a bare curl, which is the verification recipe', async () => {
    // docs/delivery-checklist.md §3a checks a deploy with `curl`, which may
    // send no Origin at all. That endpoint hands out liveness, not a socket.
    const res = await worker.fetch(
      new Request('https://phoenix-rendezvous.workers.dev/v1/health'),
      env(),
    );
    expect(res.status).toBe(200);
    const body = await res.json();
    expect(body.ok).toBe(true);
    expect(body.protocol).toBeGreaterThan(0);
  });

  it('tells /v1/health whether the origin it was given is allowed', async () => {
    // The whole point of the endpoint: an upgrade refusal is invisible from
    // the page's side, so the operator asks here after every origin change.
    const ask = async (origin) => {
      const res = await worker.fetch(
        new Request('https://phoenix-rendezvous.workers.dev/v1/health', {
          headers: { Origin: origin },
        }),
        env(),
      );
      return (await res.json()).origin_allowed;
    };
    expect(await ask(ALLOWED)).toBe(true);
    expect(await ask('https://evil.test')).toBe(false);
  });

  it('does not upgrade a plain GET, whatever its origin', async () => {
    const res = await worker.fetch(
      new Request('https://phoenix-rendezvous.workers.dev/v1/join', {
        headers: { Origin: ALLOWED },
      }),
      env(),
    );
    expect(res.status).toBe(426);
  });
});
