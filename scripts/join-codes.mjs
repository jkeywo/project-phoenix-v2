// scripts/join-codes.mjs — generate assets/join/join-codes.json from the
// authored assets/join/join-codes.toml (issue #1111).
//
// The TOML is what a designer edits (AGENTS.md rule 11): the deny-list, the
// alphabet, the confusable map and the registry bounds all carry real comments
// there. The JSON beside it is a COMMITTED generated artifact, because the
// three consumers of the table cannot read TOML:
//
//   - the phone fetches assets/join/join-codes.json as plain JSON; the client
//     page ships as unbundled ES modules, so it has no parser to reach for;
//   - worker-rendezvous/src/index.js `import`s it, and esbuild bundles JSON
//     but not TOML;
//   - the vitest suites read it from disk, and pin the shipped table rather
//     than a fixture copy of it.
//
// scripts/build-client.mjs regenerates the JSON on every client build, and
// tests/client/join-codes-data.test.js fails when the committed JSON has
// drifted from the TOML — the same drift discipline as `npm run lods:check`.

import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { parse } from 'smol-toml';

/** Where each file lives, relative to the repo root. */
export const JOIN_CODES_TOML = path.join('assets', 'join', 'join-codes.toml');
export const JOIN_CODES_JSON = path.join('assets', 'join', 'join-codes.json');

/**
 * The header the generated file leads with. JSON cannot carry a comment, so
 * the one thing the file must say about itself — do not edit me — is a value.
 */
const NOTE = [
  'GENERATED FILE — edit assets/join/join-codes.toml, not this one.',
  'Regenerate with `node scripts/build-client.mjs`; tests/client/join-codes-data.test.js',
  'fails when this file and the TOML have drifted apart.',
  '',
  'It is committed rather than built on demand because its three consumers read',
  'it with no parser and no build step: the phone fetches it, the vitest suites',
  'read it from disk, and esbuild bundles it into the rendezvous Worker.',
];

/**
 * The JSON text for the authored table, exactly as it should appear on disk.
 *
 * Field order follows the TOML document, so a reordering of the source shows up
 * as a diff in the artifact rather than silently not mattering.
 *
 * @param {string} root repo root
 * @returns {Promise<string>} file contents including the trailing newline
 */
export async function joinCodesJson(root) {
  const toml = await readFile(path.join(root, JOIN_CODES_TOML), 'utf8');
  const data = parse(toml);
  return `${JSON.stringify({ note: NOTE, ...data }, null, 2)}\n`;
}
