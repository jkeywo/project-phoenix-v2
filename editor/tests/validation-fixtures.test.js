/**
 * validation-fixtures.test.js — Round-trip the canonical world fixtures
 * (assets/worlds/default.toml and assets/worlds/patrol.toml) through
 * smol-toml parse + validateFile and assert they are clean.
 *
 * The synthesised broken-world cases that used to live here pinned the
 * `[[trigger]]` / `[[comms]]` cross-reference validator, deleted with the
 * declarative scenario front-end (issue #985). A scripted world's entity
 * references are checked by the script diagnostics, not by `validateFile`.
 */
import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';
import { parse } from 'smol-toml';
import { validateFile } from '../validation.js';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const repoRoot = resolve(__dirname, '..', '..');

function loadWorld(relPath) {
  const text = readFileSync(resolve(repoRoot, relPath), 'utf8');
  return parse(text);
}

describe('validation against shipped world fixtures', () => {
  it('assets/worlds/default.toml validates clean', () => {
    const parsed = loadWorld('assets/worlds/default.toml');
    const results = validateFile('assets/worlds/default.toml', parsed);
    // Surface the actual messages on failure so the diff is debuggable.
    expect(results).toEqual([]);
  });

  it('assets/worlds/patrol.toml validates clean', () => {
    const parsed = loadWorld('assets/worlds/patrol.toml');
    const results = validateFile('assets/worlds/patrol.toml', parsed);
    expect(results).toEqual([]);
  });
});

describe('validation surfaces composed errors', () => {
  it('flags a world missing [global] and [anchors]', () => {
    const results = validateFile('assets/worlds/broken.toml', { entity: [] });
    expect(results.some(r => r.severity === 'error' && /\[global\]/.test(r.message))).toBe(true);
    expect(results.some(r => r.severity === 'error' && /\[anchors\]/.test(r.message))).toBe(true);
  });
});
