import { describe, it, expect } from 'vitest';
import { parse as tomlParse } from 'smol-toml';
import {
  exportModPack,
  isAllowedContentPath,
  validateManifestEntries,
  buildManifestToml,
  createStoreZip,
  readStoreZip,
  crc32,
  MANIFEST_PATH,
} from '../mod-pack-export.js';

// Issue #759 — validated TOML mod-pack export.
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
    // The manifest is always present and parseable.
    expect(files[MANIFEST_PATH]).toBeDefined();
    expect(tomlParse(files[MANIFEST_PATH]).scenario[0].id).toBe('default');
  });

  it('AC1: a disallowed path blocks the export', () => {
    const result = exportModPack({
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
      files: [
        {
          path: DEFAULT_WORLD_PATH,
          // Structurally valid, but a dangling trigger reference (warning only).
          parsed: goodWorld({
            entity: [{ name: 'real' }],
            trigger: [{ condition: 'on_destroyed', entity: 'phantom' }],
          }),
        },
      ],
      scenarios: [{ id: 'default', world: DEFAULT_WORLD_PATH }],
    });
    expect(result.ok).toBe(true);
    expect(result.warnings.some((w) => w.includes('phantom'))).toBe(true);
  });

  it('AC2: manifest root-world validated against selected content — missing world blocks', () => {
    const result = exportModPack({
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
      files: [{ path: DEFAULT_WORLD_PATH, parsed: goodWorld() }],
      scenarios: [],
    });
    expect(result.ok).toBe(false);
    expect(result.errors.some((e) => e.includes('no [[scenario]] entries'))).toBe(true);
  });

  it('AC5: the produced archive is a parseable store-only ZIP whose manifest re-parses', () => {
    const result = exportModPack({
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
  });

  it('rejects a caller-supplied scenarios.toml among the selected files', () => {
    const result = exportModPack({
      files: [
        { path: MANIFEST_PATH, parsed: { scenario: [] } },
        { path: DEFAULT_WORLD_PATH, parsed: goodWorld() },
      ],
      scenarios: [{ id: 'default', world: DEFAULT_WORLD_PATH }],
    });
    expect(result.ok).toBe(false);
    expect(result.errors.some((e) => e.includes('generated by the exporter'))).toBe(true);
  });
});
