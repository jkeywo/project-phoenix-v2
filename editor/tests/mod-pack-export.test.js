import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { parse as tomlParse } from 'smol-toml';
import {
  exportModPack,
  isAllowedContentPath,
  siblingScriptPath,
  validateManifestEntries,
  validatePackMeta,
  buildManifestToml,
  buildPackTable,
  createStoreZip,
  readStoreZip,
  crc32,
  PACK_FORMAT,
  MANIFEST_PATH,
} from '../mod-pack-export.js';

// Issue #759 — validated TOML mod-pack export.
// Issue #986 — required [pack] identity header + content-compatibility gate.
//
// The exporter is the export twin of the SaveFlow admission gate (issue #757):
// definite errors across ALL selected files (and the manifest) block the
// export; warnings stay visible but never block. The produced archive is a
// TOML-only, store-only ZIP carrying the required scenario manifest and only
// whitelisted authored paths.

// A structurally valid world: has [global] and [anchors], so validateFile
// produces no error findings.
function goodWorld(extra = {}) {
  return { global: {}, anchors: {}, ...extra };
}

// A structurally valid entity: non-empty tags + a shape.
function goodEntity() {
  return { tags: ['ship'], shape: { kind: 'sphere', radius: 1 } };
}

// Well-formed [pack] identity metadata (issue #986). Every successful export
// needs it; the identity-rejection tests below vary or omit it.
function goodPack(extra = {}) {
  return {
    format: 1,
    id: 'test-pack',
    version: '1.0.0',
    name: 'Test Pack',
    requires: { content_id: 'phoenix-base', content_epoch: 1 },
    ...extra,
  };
}

const DEFAULT_WORLD_PATH = 'assets/worlds/default.toml';

describe('isAllowedContentPath (AC1 whitelist)', () => {
  it('accepts supported authored TOML paths and the manifest', () => {
    expect(isAllowedContentPath('assets/worlds/default.toml')).toBe(true);
    expect(isAllowedContentPath('assets/entities/cruiser.toml')).toBe(true);
    expect(isAllowedContentPath('assets/factions/alliance.toml')).toBe(true);
    expect(isAllowedContentPath('assets/models/cruiser.model.toml')).toBe(true);
    expect(isAllowedContentPath('scenarios.toml')).toBe(true);
  });

  it('rejects anything outside the supported authored paths', () => {
    expect(isAllowedContentPath('assets/worlds/nested/x.toml')).toBe(false);
    expect(isAllowedContentPath('assets/scripts/x.toml')).toBe(false);
    expect(isAllowedContentPath('assets/models/mesh.glb')).toBe(false);
    expect(isAllowedContentPath('assets/worlds/x.json')).toBe(false);
    expect(isAllowedContentPath('assets/worlds/.toml')).toBe(false);
    expect(isAllowedContentPath('../secret.toml')).toBe(false);
    expect(isAllowedContentPath('assets/worlds/..\\x.toml')).toBe(false);
    expect(isAllowedContentPath('')).toBe(false);
    expect(isAllowedContentPath(null)).toBe(false);
  });

  it('#988: accepts a sibling assets/worlds/*.rhai and rejects .rhai elsewhere', () => {
    expect(isAllowedContentPath('assets/worlds/combat.rhai')).toBe(true);
    // Wrong directory, nested, empty stem, or traversal — all rejected.
    expect(isAllowedContentPath('assets/entities/x.rhai')).toBe(false);
    expect(isAllowedContentPath('assets/worlds/sub/x.rhai')).toBe(false);
    expect(isAllowedContentPath('assets/worlds/.rhai')).toBe(false);
    expect(isAllowedContentPath('combat.rhai')).toBe(false);
    expect(isAllowedContentPath('assets/worlds/../x.rhai')).toBe(false);
  });

  it('#988: siblingScriptPath resolves relative to the world file (mirrors load.rs)', () => {
    expect(siblingScriptPath('assets/worlds/combat_test.toml', 'combat.rhai')).toBe(
      'assets/worlds/combat.rhai',
    );
    expect(siblingScriptPath('assets/worlds/combat_test.toml', 'sub\\combat.rhai')).toBe(
      'assets/worlds/sub/combat.rhai',
    );
  });
});

describe('store-only ZIP writer', () => {
  it('crc32 matches a known IEEE value', () => {
    // CRC-32 of the ASCII bytes "123456789" is 0xCBF43926.
    expect(crc32(new TextEncoder().encode('123456789'))).toBe(0xcbf43926);
  });

  it('round-trips entries back to their text (AC5 parseable archive)', () => {
    const entries = [
      { path: 'scenarios.toml', text: '[[scenario]]\nid = "a"\n' },
      { path: 'assets/worlds/default.toml', text: '[global]\ntitle = "x"\n' },
    ];
    const zip = createStoreZip(entries);
    expect(zip).toBeInstanceOf(Uint8Array);
    // ZIP local-file-header magic.
    expect(zip[0]).toBe(0x50);
    expect(zip[1]).toBe(0x4b);

    const files = readStoreZip(zip);
    expect(Object.keys(files).sort()).toEqual([
      'assets/worlds/default.toml',
      'scenarios.toml',
    ]);
    expect(files['scenarios.toml']).toBe('[[scenario]]\nid = "a"\n');
    expect(files['assets/worlds/default.toml']).toBe('[global]\ntitle = "x"\n');
  });

  it('a corrupted archive fails CRC verification', () => {
    const zip = createStoreZip([{ path: 'a.toml', text: 'hello world' }]);
    // Data begins after the 30-byte local header + 6-byte name "a.toml".
    zip[30 + 6] ^= 0xff;
    expect(() => readStoreZip(zip)).toThrow(/CRC/);
  });
});

describe('buildManifestToml + validateManifestEntries', () => {
  it('serialises [[scenario]] entries that parse back to the same schema', () => {
    const toml = buildManifestToml([
      { id: 'default', world: DEFAULT_WORLD_PATH, label: 'Default' },
      { id: 'combat', world: 'assets/worlds/combat.toml' },
    ]);
    const parsed = tomlParse(toml);
    expect(parsed.scenario).toHaveLength(2);
    expect(parsed.scenario[0]).toEqual({
      id: 'default',
      world: DEFAULT_WORLD_PATH,
      label: 'Default',
    });
    expect(parsed.scenario[1]).toEqual({
      id: 'combat',
      world: 'assets/worlds/combat.toml',
    });
    // With no pack argument there is no [pack] header (base-manifest shape).
    expect(parsed.pack).toBeUndefined();
  });

  it('#986: emits a [pack] header above [[scenario]] when pack metadata is given', () => {
    const toml = buildManifestToml(
      [{ id: 'default', world: DEFAULT_WORLD_PATH }],
      goodPack({ author: 'Someone', description: 'A pack.' }),
    );
    // [pack] must precede [[scenario]] textually.
    expect(toml.indexOf('[pack]')).toBeGreaterThanOrEqual(0);
    expect(toml.indexOf('[pack]')).toBeLessThan(toml.indexOf('[[scenario]]'));

    const parsed = tomlParse(toml);
    expect(parsed.pack.format).toBe(1);
    expect(parsed.pack.id).toBe('test-pack');
    expect(parsed.pack.version).toBe('1.0.0');
    expect(parsed.pack.name).toBe('Test Pack');
    expect(parsed.pack.author).toBe('Someone');
    expect(parsed.pack.description).toBe('A pack.');
    expect(parsed.pack.requires).toEqual({ content_id: 'phoenix-base', content_epoch: 1 });
    expect(parsed.scenario[0].id).toBe('default');
  });

  it('#986: buildPackTable defaults format and omits absent optional fields', () => {
    const table = buildPackTable({ id: 'x', version: '1', name: 'X', requires: {} });
    expect(table.format).toBe(PACK_FORMAT);
    expect(table).not.toHaveProperty('author');
    expect(table).not.toHaveProperty('description');
    expect(table.requires).toEqual({ content_id: '', content_epoch: 0 });
  });

  it('#986: validatePackMeta requires id, version, name and a requires clause', () => {
    expect(validatePackMeta(null).length).toBeGreaterThan(0);
    expect(validatePackMeta(goodPack())).toEqual([]);
    expect(validatePackMeta(goodPack({ id: '   ' })).some((e) => e.includes('id'))).toBe(true);
    expect(validatePackMeta(goodPack({ version: '' })).some((e) => e.includes('version'))).toBe(true);
    expect(validatePackMeta(goodPack({ name: '' })).some((e) => e.includes('name'))).toBe(true);
    expect(
      validatePackMeta(goodPack({ requires: { content_id: '', content_epoch: 1 } })).some((e) =>
        e.includes('content_id'),
      ),
    ).toBe(true);
    expect(
      validatePackMeta(goodPack({ requires: { content_id: 'phoenix-base' } })).some((e) =>
        e.includes('content_epoch'),
      ),
    ).toBe(true);
  });

  it('empty manifest is a blocking finding', () => {
    const findings = validateManifestEntries([], {});
    expect(findings).toHaveLength(1);
    expect(findings[0].category).toBe('empty-manifest');
    expect(findings[0].severity).toBe('error');
  });

  it('reports a root world that is not included in the pack (AC2 resolve_world)', () => {
    const findings = validateManifestEntries(
      [{ id: 'default', world: DEFAULT_WORLD_PATH }],
      {}, // nothing in the pack
    );
    expect(findings.map((f) => f.category)).toContain('missing-scenario-world');
  });

  it('accepts a root world resolved within the pack', () => {
    const findings = validateManifestEntries(
      [{ id: 'default', world: DEFAULT_WORLD_PATH }],
      { [DEFAULT_WORLD_PATH]: '[global]\ntitle = "x"\n' },
    );
    expect(findings).toEqual([]);
  });

  it('flags duplicate ids, empty id/world, unparseable and non-world paths', () => {
    const findings = validateManifestEntries(
      [
        { id: 'dup', world: DEFAULT_WORLD_PATH },
        { id: 'dup', world: 'assets/worlds/other.toml' },
        { id: '', world: DEFAULT_WORLD_PATH },
        { id: 'noworld', world: '' },
        { id: 'notworld', world: 'assets/entities/x.toml' },
        { id: 'broken', world: 'assets/worlds/broken.toml' },
      ],
      {
        [DEFAULT_WORLD_PATH]: '[global]\n',
        'assets/worlds/other.toml': '[global]\n',
        'assets/worlds/broken.toml': 'not valid [',
      },
    );
    const cats = findings.map((f) => f.category);
    expect(cats).toContain('duplicate-scenario-id');
    expect(cats).toContain('invalid-manifest-entry'); // empty id + empty world
    expect(cats).toContain('invalid-manifest-world-path'); // entities/ path
    expect(cats).toContain('unparseable-scenario-world');
  });
});

describe('exportModPack', () => {
  it('AC1: archive contains only allowed paths plus the required manifest', () => {
    const result = exportModPack({
      pack: goodPack(),
      files: [
        { path: DEFAULT_WORLD_PATH, parsed: goodWorld() },
        { path: 'assets/entities/cruiser.toml', parsed: goodEntity() },
      ],
      scenarios: [{ id: 'default', world: DEFAULT_WORLD_PATH }],
    });
    expect(result.ok).toBe(true);
    const files = readStoreZip(result.zip);
    const names = Object.keys(files).sort();
    expect(names).toEqual([
      'assets/entities/cruiser.toml',
      DEFAULT_WORLD_PATH,
      'scenarios.toml',
    ]);
    // The manifest is always present and parseable, with a [pack] header.
    expect(files[MANIFEST_PATH]).toBeDefined();
    const manifest = tomlParse(files[MANIFEST_PATH]);
    expect(manifest.scenario[0].id).toBe('default');
    expect(manifest.pack.id).toBe('test-pack');
  });

  it('#986: refuses the export when [pack] metadata is missing', () => {
    const result = exportModPack({
      files: [{ path: DEFAULT_WORLD_PATH, parsed: goodWorld() }],
      scenarios: [{ id: 'default', world: DEFAULT_WORLD_PATH }],
      // No pack.
    });
    expect(result.ok).toBe(false);
    expect(result.errors.some((e) => e.includes('[pack] metadata'))).toBe(true);
    expect(result.zip).toBeUndefined();
  });

  it('#986: refuses the export when [pack] metadata is invalid (empty id)', () => {
    const result = exportModPack({
      pack: goodPack({ id: '' }),
      files: [{ path: DEFAULT_WORLD_PATH, parsed: goodWorld() }],
      scenarios: [{ id: 'default', world: DEFAULT_WORLD_PATH }],
    });
    expect(result.ok).toBe(false);
    expect(result.errors.some((e) => e.includes('id'))).toBe(true);
  });

  it('AC1: a disallowed path blocks the export', () => {
    const result = exportModPack({
      pack: goodPack(),
      files: [
        { path: DEFAULT_WORLD_PATH, parsed: goodWorld() },
        { path: 'assets/scripts/evil.toml', parsed: { global: {}, anchors: {} } },
      ],
      scenarios: [{ id: 'default', world: DEFAULT_WORLD_PATH }],
    });
    expect(result.ok).toBe(false);
    expect(result.errors.some((e) => e.includes('assets/scripts/evil.toml'))).toBe(true);
    expect(result.zip).toBeUndefined();
  });

  it('AC3: a definite authoring error blocks the export', () => {
    const result = exportModPack({
      pack: goodPack(),
      // World missing [global] and [anchors] => error findings from validateFile.
      files: [{ path: 'assets/worlds/bad.toml', parsed: { name: 'nope' } }],
      scenarios: [{ id: 'bad', world: 'assets/worlds/bad.toml' }],
    });
    expect(result.ok).toBe(false);
    expect(result.errors.length).toBeGreaterThan(0);
    expect(result.errors.some((e) => e.startsWith('assets/worlds/bad.toml:'))).toBe(true);
  });

  it('AC3: warnings do NOT block the export and are surfaced', () => {
    const result = exportModPack({
      pack: goodPack(),
      files: [
        { path: DEFAULT_WORLD_PATH, parsed: goodWorld({ entity: [{ name: 'real' }] }) },
        {
          path: 'assets/entities/warn.toml',
          // Structurally valid, but `initial_state` names no declared
          // state (warning only).
          parsed: {
            ...goodEntity(),
            behaviour: { state: [{ name: 'idle' }], initial_state: 'phantom' },
          },
        },
      ],
      scenarios: [{ id: 'default', world: DEFAULT_WORLD_PATH }],
    });
    expect(result.ok).toBe(true);
    expect(result.warnings.some((w) => w.includes('phantom'))).toBe(true);
  });

  it('AC2: manifest root-world validated against selected content — missing world blocks', () => {
    const result = exportModPack({
      pack: goodPack(),
      files: [{ path: DEFAULT_WORLD_PATH, parsed: goodWorld() }],
      // References a world NOT among the selected files.
      scenarios: [{ id: 'ghost', world: 'assets/worlds/ghost.toml' }],
    });
    expect(result.ok).toBe(false);
    expect(
      result.errors.some((e) => e.includes('ghost.toml') && e.includes('not included')),
    ).toBe(true);
  });

  it('AC2: an empty manifest blocks the export', () => {
    const result = exportModPack({
      pack: goodPack(),
      files: [{ path: DEFAULT_WORLD_PATH, parsed: goodWorld() }],
      scenarios: [],
    });
    expect(result.ok).toBe(false);
    expect(result.errors.some((e) => e.includes('no [[scenario]] entries'))).toBe(true);
  });

  it('AC5: the produced archive is a parseable store-only ZIP whose manifest re-parses', () => {
    const result = exportModPack({
      pack: goodPack(),
      files: [{ path: DEFAULT_WORLD_PATH, parsed: goodWorld() }],
      scenarios: [{ id: 'default', world: DEFAULT_WORLD_PATH, label: 'Default' }],
    });
    expect(result.ok).toBe(true);
    const files = readStoreZip(result.zip);
    // Every archived file is valid TOML.
    for (const text of Object.values(files)) {
      expect(() => tomlParse(text)).not.toThrow();
    }
    const manifest = tomlParse(files[MANIFEST_PATH]);
    expect(manifest.scenario[0]).toEqual({
      id: 'default',
      world: DEFAULT_WORLD_PATH,
      label: 'Default',
    });
    expect(manifest.pack.name).toBe('Test Pack');
  });

  // ── Composable-template dependencies (issue #910) ─────────────────────────

  const FRAGMENT_PATH = 'assets/entities/base.toml';
  const HULL_PATH = 'assets/entities/hull.toml';
  const FRAGMENT_TEXT =
    'tags = ["ship"]\n[[system]]\nid = "helm-thrust"\nkind = "helm_thrust"\n';

  it('#910: carries a composed hull\'s fragment into the pack as a dependency', () => {
    const result = exportModPack({
      pack: goodPack(),
      files: [
        { path: DEFAULT_WORLD_PATH, parsed: goodWorld() },
        // The hull authors ONLY its includes — tags + systems come from the
        // fragment, which is NOT itself a selected file.
        { path: HULL_PATH, parsed: { includes: ['base.toml'] } },
      ],
      scenarios: [{ id: 'default', world: DEFAULT_WORLD_PATH }],
      fragmentSource: { [FRAGMENT_PATH]: FRAGMENT_TEXT },
    });
    expect(result.ok).toBe(true);
    const files = readStoreZip(result.zip);
    // The fragment is carried so the exported pack does not reference a
    // fragment it lacks.
    expect(Object.keys(files)).toContain(FRAGMENT_PATH);
    expect(Object.keys(files)).toContain(HULL_PATH);
    // Every archived file (fragment included) is valid TOML.
    for (const text of Object.values(files)) {
      expect(() => tomlParse(text)).not.toThrow();
    }
  });

  it('#910: validates the RESOLVED hull — tags from the fragment satisfy the check', () => {
    // Unresolved, the hull has no `tags` and would fail validateEntityToml
    // ("must have at least one tag"). It exports cleanly ONLY because the
    // exporter validates the resolved document, whose tags come from the
    // fragment.
    const result = exportModPack({
      pack: goodPack(),
      files: [
        { path: DEFAULT_WORLD_PATH, parsed: goodWorld() },
        { path: HULL_PATH, parsed: { includes: ['base.toml'] } },
      ],
      scenarios: [{ id: 'default', world: DEFAULT_WORLD_PATH }],
      fragmentSource: { [FRAGMENT_PATH]: FRAGMENT_TEXT },
    });
    expect(result.ok).toBe(true);
    expect(result.errors).toBeUndefined();
  });

  it('#910: a hull whose fragment is missing blocks the export, naming the hull', () => {
    const result = exportModPack({
      pack: goodPack(),
      files: [
        { path: DEFAULT_WORLD_PATH, parsed: goodWorld() },
        { path: HULL_PATH, parsed: { includes: ['base.toml'] } },
      ],
      scenarios: [{ id: 'default', world: DEFAULT_WORLD_PATH }],
      // No fragmentSource — the fragment cannot be resolved.
    });
    expect(result.ok).toBe(false);
    expect(
      result.errors.some((e) => e.includes(HULL_PATH) && e.includes('base.toml')),
    ).toBe(true);
    expect(result.zip).toBeUndefined();
  });

  it('rejects a caller-supplied scenarios.toml among the selected files', () => {
    const result = exportModPack({
      pack: goodPack(),
      files: [
        { path: MANIFEST_PATH, parsed: { scenario: [] } },
        { path: DEFAULT_WORLD_PATH, parsed: goodWorld() },
      ],
      scenarios: [{ id: 'default', world: DEFAULT_WORLD_PATH }],
    });
    expect(result.ok).toBe(false);
    expect(result.errors.some((e) => e.includes('generated by the exporter'))).toBe(true);
  });

  // ── Rhai script members (issue #988) ──────────────────────────────────────
  // The exporter admits a `.rhai` STRUCTURALLY — non-empty text + referenced by
  // a world — but never compiles it; the host sandbox is the authoritative gate.

  const SCRIPT_WORLD_PATH = 'assets/worlds/combat.toml';
  const SCRIPT_PATH = 'assets/worlds/combat.rhai';

  it('#988: carries a .rhai referenced by a world, verbatim, into the archive', () => {
    const scriptText = 'fn on_alarm(ctx) { let n = 2 + 2; n }\n';
    const result = exportModPack({
      pack: goodPack(),
      files: [
        { path: SCRIPT_WORLD_PATH, parsed: goodWorld({ script: 'combat.rhai' }) },
        { path: SCRIPT_PATH, text: scriptText },
      ],
      scenarios: [{ id: 'combat', world: SCRIPT_WORLD_PATH }],
    });
    expect(result.ok).toBe(true);
    const files = readStoreZip(result.zip);
    // The script rides along verbatim (NOT parsed or reserialised as TOML).
    expect(files[SCRIPT_PATH]).toBe(scriptText);
    expect(Object.keys(files)).toContain(SCRIPT_WORLD_PATH);
  });

  it('#988: rejects an empty .rhai script', () => {
    const result = exportModPack({
      pack: goodPack(),
      files: [
        { path: SCRIPT_WORLD_PATH, parsed: goodWorld({ script: 'combat.rhai' }) },
        { path: SCRIPT_PATH, text: '   \n' },
      ],
      scenarios: [{ id: 'combat', world: SCRIPT_WORLD_PATH }],
    });
    expect(result.ok).toBe(false);
    expect(result.errors.some((e) => e.includes(SCRIPT_PATH) && e.includes('empty'))).toBe(true);
  });

  it('#988: rejects a .rhai no world references', () => {
    const result = exportModPack({
      pack: goodPack(),
      files: [
        // The world declares NO script, so the carried .rhai is an orphan.
        { path: SCRIPT_WORLD_PATH, parsed: goodWorld() },
        { path: SCRIPT_PATH, text: 'fn on_x(ctx) { }\n' },
      ],
      scenarios: [{ id: 'combat', world: SCRIPT_WORLD_PATH }],
    });
    expect(result.ok).toBe(false);
    expect(result.errors.some((e) => e.includes(SCRIPT_PATH) && e.includes('not referenced'))).toBe(
      true,
    );
  });
});

// ── Committed fixtures: the SAME bytes Rust validates (issue #986) ───────────
//
// scripts/build-mod-pack-fixtures.mjs writes these deterministically; the Rust
// side include_bytes!s them through `validate_mod_pack`. Reading them here
// through `readStoreZip` proves both languages agree on one archive.

const FIXTURE_DIR = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  '../../tests/fixtures/mod-packs',
);

function readFixtureManifest(name) {
  const bytes = new Uint8Array(readFileSync(path.join(FIXTURE_DIR, name)));
  const files = readStoreZip(bytes);
  expect(files[MANIFEST_PATH]).toBeDefined();
  return tomlParse(files[MANIFEST_PATH]);
}

describe('committed mod-pack fixtures round-trip through readStoreZip', () => {
  it('valid-v1.zip carries a well-formed [pack] header', () => {
    const manifest = readFixtureManifest('valid-v1.zip');
    expect(manifest.pack.format).toBe(1);
    expect(manifest.pack.id).toBe('aurora-skirmish');
    expect(manifest.pack.version).toBe('1.0.0');
    expect(manifest.pack.name).toBe('Aurora Skirmish');
    expect(manifest.pack.requires).toEqual({ content_id: 'phoenix-base', content_epoch: 1 });
    expect(manifest.scenario[0].world).toBe('assets/worlds/aurora_skirmish.toml');
  });

  it('format-too-new.zip declares a format above the supported max', () => {
    const manifest = readFixtureManifest('format-too-new.zip');
    expect(manifest.pack.format).toBe(2);
    expect(manifest.pack.format).toBeGreaterThan(PACK_FORMAT);
  });

  it('content-epoch-mismatch.zip requires a mismatched content epoch', () => {
    const manifest = readFixtureManifest('content-epoch-mismatch.zip');
    expect(manifest.pack.format).toBe(1);
    expect(manifest.pack.requires.content_epoch).toBe(2);
  });

  // The overlapping pair (issue #987): both packs carry the SAME authored path
  // with DISTINCT content, which is what drives the Rust-side
  // `overlapping-pack-path` warning (later loaded wins).
  it('overlap-a.zip and overlap-b.zip carry the same arena world path with distinct content', () => {
    const a = readStoreZip(new Uint8Array(readFileSync(path.join(FIXTURE_DIR, 'overlap-a.zip'))));
    const b = readStoreZip(new Uint8Array(readFileSync(path.join(FIXTURE_DIR, 'overlap-b.zip'))));
    const shared = 'assets/worlds/shared_arena.toml';
    expect(a[shared]).toBeDefined();
    expect(b[shared]).toBeDefined();
    expect(a[shared]).not.toBe(b[shared]);
    expect(tomlParse(a[MANIFEST_PATH]).pack.id).toBe('overlap-a');
    expect(tomlParse(b[MANIFEST_PATH]).pack.id).toBe('overlap-b');
  });

  // The script-carrying pair (issue #988): both declare a sibling `.rhai` and
  // carry it under assets/worlds/. Rust compiles them under the sandbox; here we
  // only prove the SAME bytes round-trip and the script member is present.
  it('script-valid.zip carries a referenced sibling .rhai under assets/worlds/', () => {
    const files = readStoreZip(
      new Uint8Array(readFileSync(path.join(FIXTURE_DIR, 'script-valid.zip'))),
    );
    expect(tomlParse(files[MANIFEST_PATH]).pack.id).toBe('script-valid');
    const world = tomlParse(files['assets/worlds/script_valid.toml']);
    // `script` is a TOP-LEVEL world key (the layout the loader reads).
    expect(world.script).toBe('script_valid.rhai');
    // The .rhai rides along as raw text (it is NOT valid TOML).
    expect(files['assets/worlds/script_valid.rhai']).toContain('fn on_alarm');
    expect(isAllowedContentPath('assets/worlds/script_valid.rhai')).toBe(true);
  });

  it('script-denied-capability.zip carries the wall-clock script the host rejects', () => {
    const files = readStoreZip(
      new Uint8Array(readFileSync(path.join(FIXTURE_DIR, 'script-denied-capability.zip'))),
    );
    expect(tomlParse(files[MANIFEST_PATH]).pack.id).toBe('script-denied-capability');
    expect(files['assets/worlds/script_denied.rhai']).toContain('timestamp()');
  });
});
