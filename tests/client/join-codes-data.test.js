// The authored join-code table and the artifact generated from it (issue #1111).
//
// assets/join/join-codes.toml is the source a designer edits; the JSON beside
// it is committed because its three consumers read it with no parser and no
// build step (the phone fetches it, these suites read it from disk, esbuild
// bundles it into the rendezvous Worker). A committed generated file is only
// safe with a drift gate, so this is that gate — the same discipline as
// `npm run lods:check` applies to the LOD manifest.

import { describe, it, expect } from 'vitest';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { joinCodesJson, JOIN_CODES_JSON } from '../../scripts/join-codes.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');

describe('authored join-code data', () => {
  it('has a committed JSON artifact identical to a fresh generation from the TOML', async () => {
    const committed = await readFile(path.join(root, JOIN_CODES_JSON), 'utf8');
    const generated = await joinCodesJson(root);
    expect(
      committed,
      'assets/join/join-codes.json is stale — run `node scripts/build-client.mjs` and commit it',
    ).toBe(generated);
  });

  it('says in the artifact itself that the TOML is the file to edit', async () => {
    const data = JSON.parse(await readFile(path.join(root, JOIN_CODES_JSON), 'utf8'));
    // JSON cannot carry a comment, so the one thing it must say about itself
    // is a value. Someone will open the JSON first; it has to redirect them.
    expect(data.note.join(' ')).toContain('join-codes.toml');
    expect(data.note.join(' ')).toMatch(/GENERATED/i);
  });

  it('carries the designer-editable content in the TOML, with real comments', async () => {
    const toml = await readFile(path.join(root, 'assets/join/join-codes.toml'), 'utf8');
    // The deny-list is the part of this table AGENTS.md rule 11 is really
    // about — a designer edits it, and in TOML the rationale beside each
    // decision can be a comment rather than a data field pretending to be one.
    expect(toml).toContain('[[deny]]');
    expect(toml.split('\n').filter((l) => l.trim().startsWith('#')).length).toBeGreaterThan(20);
  });
});
