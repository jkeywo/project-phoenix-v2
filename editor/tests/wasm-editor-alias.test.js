/**
 * wasm-editor-alias.test.js — the Trunk post_build step that writes the stable
 * `dist/phoenix.js` alias the editor imports (issue #995). Exercises the glob
 * (newest-wins, alias excluded) + the shim source shape + a real round-trip to
 * a temp dir, so a URL/re-export regression in the build glue is caught without
 * a multi-minute `trunk build`.
 */
import { describe, it, expect } from 'vitest';
import { mkdtempSync, writeFileSync, readFileSync, utimesSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { findHashedGlue, aliasSource, writeAlias } from '../../scripts/wasm-editor-alias.mjs';

describe('findHashedGlue', () => {
  it('picks the phoenix glue and ignores unrelated files + the alias itself', () => {
    const files = [
      'phoenix.js', // the alias we write — must never be chosen as the source
      'project-phoenix-9b074c85c4a6a2d7.js',
      'project-phoenix-9b074c85c4a6a2d7_bg.wasm',
      'index.html',
      'snippets', // a directory name, .js filter excludes it anyway
    ];
    const mtimes = { 'project-phoenix-9b074c85c4a6a2d7.js': 100 };
    const readdir = () => files;
    const stat = (p) => ({ mtimeMs: mtimes[p.split(/[\\/]/).pop()] ?? 0 });
    expect(findHashedGlue('dist', readdir, stat)).toBe('project-phoenix-9b074c85c4a6a2d7.js');
  });

  it('returns the most recently written glue when stale hashes linger', () => {
    const files = ['project-phoenix-aaa.js', 'project-phoenix-bbb.js'];
    const mtimes = { 'project-phoenix-aaa.js': 10, 'project-phoenix-bbb.js': 99 };
    const readdir = () => files;
    const stat = (p) => ({ mtimeMs: mtimes[p.split(/[\\/]/).pop()] });
    expect(findHashedGlue('dist', readdir, stat)).toBe('project-phoenix-bbb.js');
  });

  it('returns undefined when no glue is present', () => {
    expect(findHashedGlue('dist', () => ['index.html'], () => ({ mtimeMs: 0 }))).toBeUndefined();
  });
});

describe('aliasSource', () => {
  const src = aliasSource('project-phoenix-9b074c85c4a6a2d7.js');

  it('re-exports the named exports of the hashed glue', () => {
    expect(src).toContain("export * from './project-phoenix-9b074c85c4a6a2d7.js'");
  });

  it('re-exports a default init bound to the hashed _bg.wasm path', () => {
    // export * does NOT carry the default, so a default must be defined here...
    expect(src).toMatch(/export default function/);
    // ...and it must fetch the HASHED wasm, not the un-hashed wasm-bindgen default.
    expect(src).toContain("new URL('./project-phoenix-9b074c85c4a6a2d7_bg.wasm', import.meta.url)");
    expect(src).not.toContain('project-phoenix_bg.wasm'); // the broken un-hashed name
  });

  it('imports init from the hashed glue so mod.default() initialises it', () => {
    expect(src).toContain("import init from './project-phoenix-9b074c85c4a6a2d7.js'");
    expect(src).toMatch(/init\(opts \?\? \{ module_or_path: WASM_URL \}\)/);
  });
});

describe('writeAlias (round-trip)', () => {
  it('discovers the newest glue and writes phoenix.js beside it', () => {
    const dir = mkdtempSync(join(tmpdir(), 'phoenix-alias-'));
    try {
      writeFileSync(join(dir, 'project-phoenix-old.js'), '// old');
      writeFileSync(join(dir, 'project-phoenix-new.js'), '// new');
      // Make -new the most recently modified so it wins the mtime sort.
      const older = new Date(Date.now() - 60_000);
      utimesSync(join(dir, 'project-phoenix-old.js'), older, older);

      const { hashedJs, outPath } = writeAlias(dir);
      expect(hashedJs).toBe('project-phoenix-new.js');
      expect(outPath).toBe(join(dir, 'phoenix.js'));

      const written = readFileSync(outPath, 'utf8');
      expect(written).toContain("export * from './project-phoenix-new.js'");
      expect(written).toContain("import init from './project-phoenix-new.js'");
      expect(written).toContain("new URL('./project-phoenix-new_bg.wasm', import.meta.url)");
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  it('throws when no glue is present (a real build failure, not a silent skip)', () => {
    const dir = mkdtempSync(join(tmpdir(), 'phoenix-alias-empty-'));
    try {
      expect(() => writeAlias(dir)).toThrow(/no project-phoenix-<hash>\.js/);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});
