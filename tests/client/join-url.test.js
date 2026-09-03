// Where a page sends its join socket (issues #1329, #1353).
//
// The rule under test is one sentence — THE SERVICE THAT SERVED YOU THE PAGE IS
// THE SERVICE YOU DIAL — and it is the whole of what makes a LAN game need no
// external service: a native host serves the bundle and accepts the join socket
// on the same port (src/native_host/direct_join.rs), so a phone that loaded
// from `http://192.168.1.5:8080` dials that. Only the origins the BROWSER game
// is published at — which cannot accept a socket — keep the cloud service.

import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';

import {
  DEV_RENDEZVOUS_URL,
  KNOWN_WEB_ORIGINS,
  joinUrlForCode,
  rendezvousBaseForOrigin,
} from '../../gui/join-url.js';

describe('rendezvousBaseForOrigin', () => {
  it('sends a page served by a native host back to that host', () => {
    // The QR points at the host's LAN address, so this is the origin every
    // phone in the room actually loads from.
    expect(rendezvousBaseForOrigin('http://192.168.1.5:8080')).toBe('http://192.168.1.5:8080');
    expect(rendezvousBaseForOrigin('http://10.0.0.9:8080')).toBe('http://10.0.0.9:8080');
    expect(rendezvousBaseForOrigin('http://[fd00::5]:8080')).toBe('http://[fd00::5]:8080');
    // A host bound to a name is the operator's own answer and is honoured as
    // one; nothing here second-guesses which interface they meant.
    expect(rendezvousBaseForOrigin('http://bridge.local:8080')).toBe('http://bridge.local:8080');
    // 127.0.0.1 is a native host reached from the machine it runs on — not one
    // of the published web origins, so it dials itself like any other host.
    expect(rendezvousBaseForOrigin('http://127.0.0.1:8080')).toBe('http://127.0.0.1:8080');
  });

  it('leaves the published web origins on the built-in service', () => {
    // These are served by static hosts that cannot accept a socket, so the
    // meet-in-the-middle service is the only thing there is to dial.
    for (const origin of KNOWN_WEB_ORIGINS) {
      expect(rendezvousBaseForOrigin(origin), origin).toBe(DEV_RENDEZVOUS_URL);
    }
    // Case and a trailing slash are the same origin, not a second one — an
    // origin that fell through those would silently swap a dev server for the
    // cloud service and the dev would be debugging the wrong host.
    expect(rendezvousBaseForOrigin('http://LOCALHOST:3000/')).toBe(DEV_RENDEZVOUS_URL);
  });

  it('falls back to the built-in service for anything that is not an origin', () => {
    // A page opened from disk, a sandboxed iframe's `null`, or no browser at
    // all (the Node test environment) was not served by anything dialable.
    for (const bad of ['', null, undefined, 'null', 'file:///C:/dist/client/index.html',
      'http://host/with/path', 'not a url', 'ws://192.168.1.5:8080']) {
      expect(rendezvousBaseForOrigin(bad), String(bad)).toBe(DEV_RENDEZVOUS_URL);
    }
  });

  it('is the list worker-rendezvous/wrangler.toml allows, and stays the list', () => {
    // The twin, and the reason the mirror is safe to keep: ALLOWED_ORIGIN is
    // the set of origins the cloud service will take a socket from, so an
    // origin missing from THIS list would be sent to a service that refuses it,
    // and an origin missing from THAT one is a page that cannot join at all.
    const wrangler = readFileSync('worker-rendezvous/wrangler.toml', 'utf8');
    const line = wrangler.match(/^ALLOWED_ORIGIN\s*=\s*"([^"]*)"/m);
    expect(line, 'wrangler.toml declares ALLOWED_ORIGIN').not.toBeNull();
    const allowed = line[1].split(',').map((s) => s.trim()).filter(Boolean);
    expect([...KNOWN_WEB_ORIGINS].sort()).toEqual([...allowed].sort());
  });
});

describe('joinUrlForCode', () => {
  it('carries no service at all when the page dials the host that served it', () => {
    // What a directly-joinable native host puts in its QR (issue #1353): the
    // page base is the host's own LAN address and the base is the default, so
    // there is no `?rendezvous=` — a shorter QR, and no parameter for a link
    // to point somewhere else.
    expect(joinUrlForCode('http://192.168.1.5:8080/', 'P_V_QUARK'))
      .toBe('http://192.168.1.5:8080/client/index.html#P_V_QUARK');
  });

  it('still names a non-default service, which is the cloud-only host case', () => {
    expect(joinUrlForCode('https://pp-dev.kiwigamedesign.co.uk/', 'P_V_QUARK', 'http://127.0.0.1:8787'))
      .toBe(
        'https://pp-dev.kiwigamedesign.co.uk/client/index.html'
          + '?rendezvous=http%3A%2F%2F127.0.0.1%3A8787#P_V_QUARK',
      );
  });
});
