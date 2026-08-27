// Cloudflare Worker — the Phoenix rendezvous/signalling service (issue #1111).
//
// A SIBLING of worker/ (the stateless TURN-credential minter), not a route on
// it: different lifetime, different state model, different failure mode. This
// one terminates secure WebSockets and holds live state, so its logic lives in
// a Durable Object.
//
// This file decides nothing. Typed code lookup, admission state, signalling
// relay, presence and code lifecycle are all in src/registry.js, which is a
// pure state machine with no socket in it — that is what lets the contract
// tests in tests/client/rendezvous-registry.test.js cover the service without
// wrangler or miniflare. Everything here is: check the Origin, upgrade the
// socket, pump frames.
//
// Endpoints (the `/v1` prefix IS the versioned interface; a later frame
// vocabulary gets /v2 rather than a flag):
//
//   GET /v1/health            JSON liveness + whether YOUR Origin is allowed
//   GET /v1/host   (Upgrade)  a ship host's socket — issues and holds a code
//   GET /v1/join   (Upgrade)  a crew client's socket — resolves and joins one
//
// Required vars (wrangler.toml [vars]):
//   ALLOWED_ORIGIN — comma-separated list of allowed site origins.
//                    THE SAME TRAP AS THE TURN WORKER, WITH WORSE
//                    CONSEQUENCES: a worker only picks up [vars] on
//                    `wrangler deploy`, so a stale deployed value here refuses
//                    every upgrade and nobody joins at all (the TURN worker's
//                    equivalent merely stripped relay). /v1/health answers
//                    `origin_allowed` for exactly this reason — see
//                    docs/delivery-checklist.md.

import format from '../../assets/join/join-codes.json';
import {
  createRegistry,
  RENDEZVOUS_PROTOCOL,
  ROLE_HOST,
  ROLE_CLIENT,
} from './registry.js';

const SERVICE = 'phoenix-rendezvous';

function allowlist(env) {
  return (env.ALLOWED_ORIGIN || '*')
    .split(',')
    .map((s) => s.trim())
    .filter(Boolean);
}

/**
 * Is this request's Origin on the list?
 *
 * `requireOrigin` is the difference between the two kinds of endpoint here.
 * A WebSocket upgrade from a browser ALWAYS carries an Origin — the pages live
 * on pp-dev/pp-demo and this service on *.workers.dev, so every legitimate
 * upgrade is cross-origin — which means an exemption for requests without one
 * exempts precisely the scripted non-browser caller the 403 exists to stop.
 * So the upgrade paths demand a present, allow-listed Origin.
 *
 * /v1/health keeps the permissive branch, because the deploy-verification
 * recipe in docs/delivery-checklist.md §3a is a bare `curl` that may send no
 * Origin at all, and that endpoint answers with liveness, not with a socket.
 */
function originAllowed(request, env, { requireOrigin = false } = {}) {
  const allowed = allowlist(env);
  if (allowed.includes('*')) return true;
  const origin = request.headers.get('Origin');
  if (!origin) return !requireOrigin;
  return allowed.includes(origin);
}

function corsHeaders(request, env) {
  const allowed = allowlist(env);
  const origin = request.headers.get('Origin');
  const allow = allowed.includes('*')
    ? '*'
    : origin && allowed.includes(origin)
      ? origin
      : allowed[0];
  return {
    'Access-Control-Allow-Origin': allow,
    'Access-Control-Allow-Methods': 'GET, OPTIONS',
    Vary: 'Origin',
  };
}

const json = (body, status, headers) =>
  new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json', 'Cache-Control': 'no-store', ...headers },
  });

/**
 * The live registry. One instance holds every record, because typed lookup has
 * to see BOTH namespaces and BOTH releases at once to tell wrong-type from
 * version-mismatch from unknown; sharding it would turn those three answers
 * back into one. Sharding by version (with a lookup fan-out) is the scaling
 * move if one instance ever becomes the ceiling.
 */
export class RendezvousRegistry {
  constructor(state, env) {
    this.env = env;
    this.registry = createRegistry({ data: format });
    /** @type {Map<string, WebSocket>} */
    this.sockets = new Map();
  }

  dispatch(frames) {
    for (const { to, frame, close } of frames) {
      const ws = this.sockets.get(to);
      if (!ws) continue;
      try {
        ws.send(JSON.stringify(frame));
      } catch {
        // A socket that has already gone away is not an error worth failing
        // the sending peer's request over; the close event cleans it up.
      }
      // The registry cannot hold a socket, so a refusal that should also END
      // the connection (a connection past its lookup cap) says so on the frame
      // and this adapter carries it out — after the refusal has been sent, so
      // the peer learns why rather than seeing an unexplained drop.
      if (!close) continue;
      try {
        ws.close(1008, frame.reason || 'refused');
      } catch {
        // Already gone; the close handler will clean up.
      }
    }
  }

  async fetch(request) {
    const url = new URL(request.url);
    const role = url.pathname.endsWith('/host') ? ROLE_HOST : ROLE_CLIENT;

    const pair = new WebSocketPair();
    const [clientSocket, serverSocket] = Object.values(pair);
    const connId = crypto.randomUUID();

    serverSocket.accept();
    this.sockets.set(connId, serverSocket);

    serverSocket.addEventListener('message', (evt) => {
      let frame = null;
      try {
        frame = JSON.parse(typeof evt.data === 'string' ? evt.data : '');
      } catch {
        frame = null;
      }
      this.dispatch(this.registry.receive(connId, frame));
    });

    const drop = () => {
      if (!this.sockets.has(connId)) return;
      this.sockets.delete(connId);
      this.dispatch(this.registry.disconnect(connId));
    };
    serverSocket.addEventListener('close', drop);
    serverSocket.addEventListener('error', drop);

    this.dispatch(this.registry.connect(connId, role));

    return new Response(null, { status: 101, webSocket: clientSocket });
  }
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);

    if (request.method === 'OPTIONS') {
      return new Response(null, { status: 204, headers: corsHeaders(request, env) });
    }
    if (request.method !== 'GET') {
      return json({ error: 'method-not-allowed' }, 405, corsHeaders(request, env));
    }

    // The deploy-verification endpoint. `curl -H "Origin: https://…"` against
    // this answers the one question the ALLOWED_ORIGIN trap makes invisible.
    if (url.pathname === '/v1/health') {
      return json(
        {
          ok: true,
          service: SERVICE,
          protocol: RENDEZVOUS_PROTOCOL,
          format_version: format.format_version,
          origin: request.headers.get('Origin') || null,
          origin_allowed: originAllowed(request, env),
        },
        200,
        corsHeaders(request, env),
      );
    }

    if (url.pathname !== '/v1/host' && url.pathname !== '/v1/join') {
      return json({ error: 'not-found' }, 404, corsHeaders(request, env));
    }

    if (request.headers.get('Upgrade') !== 'websocket') {
      return json({ error: 'expected-websocket-upgrade' }, 426, corsHeaders(request, env));
    }

    // Workers do not apply CORS to an Upgrade response, so the origin gate has
    // to be here rather than in a header. Refuse loudly and name the origin:
    // a silent refusal here is the 2026-08 TURN incident with nobody able to
    // join instead of nobody able to relay.
    if (!originAllowed(request, env, { requireOrigin: true })) {
      return json(
        {
          error: 'origin-not-allowed',
          service: SERVICE,
          origin: request.headers.get('Origin'),
        },
        403,
        corsHeaders(request, env),
      );
    }

    const id = env.RENDEZVOUS.idFromName('v1');
    return env.RENDEZVOUS.get(id).fetch(request);
  },
};
