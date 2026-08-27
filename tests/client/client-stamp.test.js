// The client bundle's build-time delivery stamp (issue #1111).
//
// Reads the REAL src/core/messages.rs and assets/scenarios.toml, so a
// PROTOCOL_VERSION bump or a [content] epoch bump that stops reaching the
// client page fails here rather than at a player's phone.

import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  protocolVersionFrom,
  contentIdentityFrom,
  stampField,
  clientStampField,
} from '../../scripts/client-stamp.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const read = (rel) => readFileSync(path.join(root, rel), 'utf8');

describe('reading the repository sources', () => {
  it('finds the protocol version the wire vocabulary declares', () => {
    const protocol = protocolVersionFrom(read('src/core/messages.rs'));
    expect(Number.isInteger(protocol)).toBe(true);
    expect(protocol).toBeGreaterThan(0);
  });

  it('finds the served content identity in the scenario manifest', () => {
    const content = contentIdentityFrom(read('assets/scenarios.toml'));
    expect(content.id).toBeTruthy();
    expect(Number.isInteger(content.epoch)).toBe(true);
  });

  it('agrees with the demo manifest, so one stamp serves both builds', () => {
    expect(contentIdentityFrom(read('assets/scenarios.demo.toml')))
      .toEqual(contentIdentityFrom(read('assets/scenarios.toml')));
  });

  it('assembles the same three-part field the HTTP header carries', async () => {
    const field = await clientStampField(root);
    expect(field.split('/')).toHaveLength(3);
    const [protocol, id, epoch] = field.split('/');
    expect(Number(protocol)).toBe(protocolVersionFrom(read('src/core/messages.rs')));
    expect(id).toBe(contentIdentityFrom(read('assets/scenarios.toml')).id);
    expect(Number(epoch)).toBe(contentIdentityFrom(read('assets/scenarios.toml')).epoch);
  });
});

describe('degrading', () => {
  it('is empty — meaning unstamped, which the host admits — when a source is unreadable', async () => {
    expect(await clientStampField(path.join(root, 'no-such-checkout'))).toBe('');
  });

  it('is empty rather than half-built when either half is missing', () => {
    expect(stampField(null, { id: 'x', epoch: 1 })).toBe('');
    expect(stampField(1, null)).toBe('');
    expect(contentIdentityFrom('[[scenario]]\nid = "x"\n')).toBeNull();
    expect(contentIdentityFrom('[content]\nid = "x"\n')).toBeNull();
  });
});

describe('the page carries a placeholder to stamp', () => {
  it('client.html has the meta tag the build rewrites', () => {
    expect(read('client.html')).toMatch(/<meta\s+name="phoenix-client-stamp"\s+content="[^"]*"/);
  });
});
