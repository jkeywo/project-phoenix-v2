import { describe, it, expect } from 'vitest';
import { parse as tomlParse } from 'smol-toml';
import {
  ModPackWorkspace,
  classifyMember,
  digestText,
} from '../mod-pack-workspace.js';
import {
  exportModPack,
  readStoreZip,
  buildManifestToml,
  parsePackManifest,
  MANIFEST_PATH,
  PACK_FORMAT,
} from '../mod-pack-export.js';

// Issue #989 — the PURE mod-pack workspace behind MOD mode. No DOM, no IO:
// metadata + scenarios + a member set, per-member new/patch classification
// against a supplied base-file map, base digest recorded at add-time, and a
// non-blocking stale-patch warning when the base drifts.

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

const WORLD_PATH = 'assets/worlds/default.toml';
const WORLD_TEXT = '[global]\n[anchors]\n';

describe('classifyMember + digestText', () => {
  it('classifies a path present in the base map as patch, absent as new', () => {
    expect(classifyMember(WORLD_PATH, { [WORLD_PATH]: WORLD_TEXT })).toBe('patch');
    expect(classifyMember(WORLD_PATH, {})).toBe('new');
    expect(classifyMember('assets/worlds/brand_new.toml', { [WORLD_PATH]: WORLD_TEXT })).toBe('new');
  });

  it('digestText is stable per content and changes when the content changes', () => {
    expect(digestText('hello')).toBe(digestText('hello'));
    expect(digestText('hello')).not.toBe(digestText('hello!'));
  });
});

describe('ModPackWorkspace metadata', () => {
  it('defaults format to PACK_FORMAT and leaves content_epoch unset (null)', () => {
    const ws = new ModPackWorkspace();
    const p = ws.getPack();
    expect(p.format).toBe(PACK_FORMAT);
    expect(p.requires.content_epoch).toBeNull();
    expect(p.id).toBe('');
  });

  it('setPack merges a partial patch; omitted fields are preserved, empty string takes', () => {
    const ws = new ModPackWorkspace();
    ws.setPack({ id: 'aurora', name: 'Aurora' });
    ws.setPack({ requires: { content_id: 'phoenix-base', content_epoch: 3 } });
    expect(ws.getPack().id).toBe('aurora');
    expect(ws.getPack().name).toBe('Aurora');
    expect(ws.getPack().requires.content_epoch).toBe(3);
    // Clearing name with '' takes; id is untouched (omitted).
    ws.setPack({ name: '' });
    expect(ws.getPack().name).toBe('');
    expect(ws.getPack().id).toBe('aurora');
  });

  it('getPack returns a defensive copy (requires is not shared)', () => {
    const ws = new ModPackWorkspace();
    const a = ws.getPack();
    a.requires.content_id = 'mutated';
    expect(ws.getPack().requires.content_id).toBe('');
  });
});

describe('ModPackWorkspace members: add/remove/classify + base digest', () => {
  it('adds a new member (path not under root) with no base digest', () => {
    const ws = new ModPackWorkspace();
    const m = ws.addMember({ path: WORLD_PATH, text: WORLD_TEXT }, {});
    expect(m.classification).toBe('new');
    expect(m.baseDigest).toBeNull();
    expect(ws.memberCount()).toBe(1);
    expect(ws.hasMember(WORLD_PATH)).toBe(true);
  });

  it('a path existing under the project root classifies as patch and stores the base digest', () => {
    const ws = new ModPackWorkspace();
    const m = ws.addMember({ path: WORLD_PATH, text: WORLD_TEXT }, { [WORLD_PATH]: WORLD_TEXT });
    expect(m.classification).toBe('patch');
    expect(m.baseDigest).toBe(digestText(WORLD_TEXT));
  });

  it('removeMember drops the member', () => {
    const ws = new ModPackWorkspace();
    ws.addMember({ path: WORLD_PATH, text: WORLD_TEXT }, {});
    expect(ws.removeMember(WORLD_PATH)).toBe(true);
    expect(ws.hasMember(WORLD_PATH)).toBe(false);
    expect(ws.removeMember(WORLD_PATH)).toBe(false);
  });

  it('rejects a member with no path', () => {
    const ws = new ModPackWorkspace();
    expect(() => ws.addMember({ text: 'x' }, {})).toThrow(/path/);
  });

  it('re-adding a path replaces it and preserves insertion position', () => {
    const ws = new ModPackWorkspace();
    ws.addMember({ path: 'a.toml', text: '1' }, {});
    ws.addMember({ path: 'b.toml', text: '2' }, {});
    ws.addMember({ path: 'a.toml', text: '3' }, {});
    const paths = ws.getMembers().map((m) => m.path);
    expect(paths).toEqual(['a.toml', 'b.toml']);
    expect(ws.getMember('a.toml').text).toBe('3');
  });
});

describe('ModPackWorkspace scenarios', () => {
  it('adds, lists, and removes scenario entries by id', () => {
    const ws = new ModPackWorkspace();
    ws.addScenario({ id: 'default', world: WORLD_PATH, label: 'Default' });
    ws.addScenario({ id: 'combat', world: 'assets/worlds/combat.toml' });
    expect(ws.getScenarios()).toEqual([
      { id: 'default', world: WORLD_PATH, label: 'Default' },
      { id: 'combat', world: 'assets/worlds/combat.toml' },
    ]);
    expect(ws.removeScenario('default')).toBe(true);
    expect(ws.getScenarios().map((s) => s.id)).toEqual(['combat']);
  });
});

describe('ModPackWorkspace staleWarnings (never blocking)', () => {
  it('warns only when a patch base drifts from the recorded digest', () => {
    const ws = new ModPackWorkspace();
    ws.addMember({ path: WORLD_PATH, text: WORLD_TEXT }, { [WORLD_PATH]: WORLD_TEXT });
    // Unchanged base — no warning.
    expect(ws.staleWarnings({ [WORLD_PATH]: WORLD_TEXT })).toEqual([]);
    // Drifted base — a single stale-patch WARNING.
    const drift = ws.staleWarnings({ [WORLD_PATH]: `${WORLD_TEXT}# edited\n` });
    expect(drift).toHaveLength(1);
    expect(drift[0].category).toBe('stale-patch');
    expect(drift[0].severity).toBe('warning');
    expect(drift[0].path).toBe(WORLD_PATH);
  });

  it('never warns for new members, and ignores a base that vanished', () => {
    const ws = new ModPackWorkspace();
    ws.addMember({ path: 'assets/worlds/brand_new.toml', text: WORLD_TEXT }, {}); // new
    ws.addMember({ path: WORLD_PATH, text: WORLD_TEXT }, { [WORLD_PATH]: WORLD_TEXT }); // patch
    // New member never warns; patch whose base is absent from the map is ignored.
    expect(ws.staleWarnings({})).toEqual([]);
  });
});

describe('ModPackWorkspace.toExportInput → exportModPack', () => {
  it('produces an input the export gate accepts, parsing TOML members', () => {
    const ws = new ModPackWorkspace({ pack: goodPack() });
    ws.addMember({ path: WORLD_PATH, text: WORLD_TEXT }, {});
    ws.addScenario({ id: 'default', world: WORLD_PATH });
    const result = exportModPack(ws.toExportInput());
    expect(result.ok).toBe(true);
    const files = readStoreZip(result.zip);
    expect(Object.keys(files).sort()).toEqual([WORLD_PATH, MANIFEST_PATH]);
    expect(tomlParse(files[MANIFEST_PATH]).pack.id).toBe('test-pack');
  });

  it('carries a .rhai member verbatim (no reserialisation)', () => {
    const ws = new ModPackWorkspace({ pack: goodPack() });
    const scriptWorld = 'assets/worlds/combat.toml';
    const scriptText = 'fn on_alarm(ctx) { 2 + 2 }\n';
    ws.addMember({ path: scriptWorld, text: 'script = "combat.rhai"\n[global]\n[anchors]\n' }, {});
    ws.addMember({ path: 'assets/worlds/combat.rhai', text: scriptText }, {});
    ws.addScenario({ id: 'combat', world: scriptWorld });
    const result = exportModPack(ws.toExportInput());
    expect(result.ok).toBe(true);
    const files = readStoreZip(result.zip);
    expect(files['assets/worlds/combat.rhai']).toBe(scriptText);
  });

  it('refuses export (via the gate) when pack metadata is incomplete', () => {
    const ws = new ModPackWorkspace(); // no id/version/name/requires
    ws.addMember({ path: WORLD_PATH, text: WORLD_TEXT }, {});
    ws.addScenario({ id: 'default', world: WORLD_PATH });
    const result = exportModPack(ws.toExportInput());
    expect(result.ok).toBe(false);
    expect(result.errors.some((e) => e.includes('[pack]'))).toBe(true);
  });
});

describe('ModPackWorkspace.fromArchiveFiles round trip', () => {
  it('re-exporting an unedited import produces BYTE-IDENTICAL archive bytes', () => {
    const ws = new ModPackWorkspace({ pack: goodPack({ author: 'Someone', description: 'A pack.' }) });
    ws.addMember({ path: WORLD_PATH, text: WORLD_TEXT }, {});
    ws.addMember({ path: 'assets/entities/cruiser.toml', text: 'tags = ["ship"]\n[shape]\nkind = "sphere"\nradius = 1\n' }, {});
    ws.addScenario({ id: 'default', world: WORLD_PATH, label: 'Default' });

    const first = exportModPack(ws.toExportInput());
    expect(first.ok).toBe(true);

    // Import the archive back and re-export WITHOUT edits.
    const files = readStoreZip(first.zip);
    const reopened = ModPackWorkspace.fromArchiveFiles(files, {});
    const second = exportModPack(reopened.toExportInput());
    expect(second.ok).toBe(true);

    expect(Array.from(second.zip)).toEqual(Array.from(first.zip));
  });

  it('fromArchiveFiles restores pack metadata + scenarios + members in archive order', () => {
    const ws = new ModPackWorkspace({ pack: goodPack() });
    ws.addMember({ path: WORLD_PATH, text: WORLD_TEXT }, {});
    ws.addScenario({ id: 'default', world: WORLD_PATH });
    const { zip } = exportModPack(ws.toExportInput());

    const reopened = ModPackWorkspace.fromArchiveFiles(readStoreZip(zip), {
      // Mark the world as an existing base file so it re-imports as a patch.
      [WORLD_PATH]: WORLD_TEXT,
    });
    expect(reopened.getPack().id).toBe('test-pack');
    expect(reopened.getScenarios()).toEqual([{ id: 'default', world: WORLD_PATH }]);
    const member = reopened.getMember(WORLD_PATH);
    expect(member.classification).toBe('patch');
    expect(member.baseDigest).toBe(digestText(WORLD_TEXT));
  });

  it('parsePackManifest is the inverse of buildManifestToml (fixed point)', () => {
    const scenarios = [{ id: 'default', world: WORLD_PATH, label: 'Default' }];
    const pack = goodPack({ author: 'A', description: 'D' });
    const toml = buildManifestToml(scenarios, pack);
    const parsed = parsePackManifest(toml);
    expect(buildManifestToml(parsed.scenarios, parsed.pack)).toBe(toml);
  });
});
