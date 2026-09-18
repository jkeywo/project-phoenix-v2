import { describe, expect, it, vi } from 'vitest';
import { createWorkshopRuntime } from '../../editor/workshop-runtime.js';
import { crc32 } from '../../editor/mod-pack-export.js';

describe('offline Workshop runtime capability', () => {
  it('previews the same immutable winning dependency bytes and refuses uncaptured delivery content', async () => {
    const sound = 'assets/sounds/pack/cue.ogg', base = 'assets/sounds/base.wav';
    const bytes = new Uint8Array([0, 255, 13, 10]);
    const source = { base_files: {},
      base_asset_manifest: { [base]: { length: bytes.length, crc32: crc32(bytes) } },
      packs: [{ assets: { [sound]: [1, 2] } }, { assets: { [sound]: [3, 4] } }] };
    const fetchAsset = vi.fn(async () => ({ ok: true, arrayBuffer: async () => bytes.buffer }));
    const runtime = createWorkshopRuntime({ dependencies: async () => source, fetchAsset,
      load: async () => ({ wasm_workshop_validate_pack: () => '{}' }) });
    const first = await runtime.readAsset(sound);
    expect(first).toEqual(new Uint8Array([3, 4]));
    first.fill(9); source.packs[1].assets[sound].fill(8);
    expect(await runtime.readAsset(sound)).toEqual(new Uint8Array([3, 4]));
    expect(await runtime.readAsset(base)).toEqual(bytes);
    expect(await runtime.readAsset('assets/sounds/undeclared.wav')).toBeNull();
    expect(fetchAsset).toHaveBeenCalledExactlyOnceWith(base);
  });

  it('passes exact candidate bytes and an isolated source snapshot without booting a host', async () => {
    const source = { base_files: { 'assets/scenarios.toml': '[content]\nid="base"\nepoch=1' }, packs: [] };
    const validate = vi.fn(() => JSON.stringify({ accepted: true, findings: [] }));
    const boot = vi.fn();
    const load = vi.fn(async () => ({ wasm_workshop_validate_pack: validate, wasm_init: boot, wasm_add_mod_pack: boot }));
    const runtime = createWorkshopRuntime({ load, dependencies: async () => source });
    const bytes = new Uint8Array([0, 25, 255]);
    expect(await runtime.validate(bytes)).toEqual({ accepted: true, findings: [] });
    source.base_files.extra = 'later mutation';
    await runtime.validate(bytes);
    expect(validate).toHaveBeenLastCalledWith(bytes, JSON.stringify({ base_files: { 'assets/scenarios.toml': '[content]\nid="base"\nepoch=1' }, packs: [], base_assets: {} }));
    expect(load).toHaveBeenCalledOnce();
    expect(boot).not.toHaveBeenCalled();
  });

  it('never treats missing or contradictory validation as acceptance and permits retry', async () => {
    const load = vi.fn().mockRejectedValueOnce(new Error('missing artifact'))
      .mockResolvedValue({ wasm_workshop_validate_pack: () => JSON.stringify({ accepted: true,
        findings: [{ severity: 'error', message: 'invalid script', file: 's.rhai' }] }) });
    const runtime = createWorkshopRuntime({ load, dependencies: async () => ({ base_files: {} }) });
    await expect(runtime.validate(new Uint8Array())).rejects.toThrow('missing artifact');
    await expect(runtime.validate(new Uint8Array())).rejects.toThrow('conflicting validation');
  });

  it('obtains field metadata and source patches from the same runtime without a JS serializer', async () => {
    const fields = [{ path: ['global', 'title'], kind: 'string', source: "'A'", line: 2 }];
    const patch = vi.fn(() => "# intact\n[global]\ntitle = 'B'\n");
    const runtime = createWorkshopRuntime({ load: async () => ({
      wasm_workshop_validate_pack: () => '{}', wasm_workshop_fields: () => JSON.stringify(fields),
      wasm_workshop_patch: patch,
    }), dependencies: async () => ({ base_files: {} }) });
    const source = "# intact\n[global]\ntitle = 'A'\n";
    expect(await runtime.inspect(source, 'assets/worlds/test.toml')).toEqual(fields);
    const change = { document_path: 'assets/worlds/test.toml', path: fields[0].path, expected_source: source, value_source: "'B'" };
    expect(await runtime.patch(source, change)).toContain("title = 'B'");
    expect(patch).toHaveBeenCalledWith(source, JSON.stringify(change));
  });

  it('reads the definition catalog with text-only dependencies and routes structural edits and new factions to the runtime', async () => {
    const source = { base_files: { 'assets/factions/alliance.toml': 'name = "Alliance"\n' },
      base_asset_manifest: { 'assets/models/a.glb': { length: 2, crc32: 0 } },
      packs: [{ id: 'raiders', manifest_toml: '[pack]\n', files: { 'assets/factions/pirate.toml': 'name = "Pirate"\n' },
        assets: { 'assets/sounds/x.ogg': [1, 2] } }] };
    const catalog = { factions: [], hulls: [], choices: { factions: [], order_responses: ['comply', 'refuse'], ai_rules: [] },
      defaults: { compliance: { ack_secs: 2 } }, findings: [] };
    const definitions = vi.fn(() => JSON.stringify(catalog));
    const edit = vi.fn(() => 'name = "Mine"\nenemies = ["b"]\n');
    const newFaction = vi.fn(() => 'uuid = "u"\nname = "Harrow"\nenemies = []\n');
    const runtime = createWorkshopRuntime({ load: async () => ({
      wasm_workshop_validate_pack: () => '{}', wasm_workshop_definitions: definitions, wasm_workshop_edit: edit,
      wasm_workshop_new_faction: newFaction,
    }), dependencies: async () => source });
    const files = { 'assets/factions/mine.toml': 'name = "Mine"\nenemies = []\n' };
    expect(await runtime.definitions(files)).toEqual(catalog);
    // Text only: no base asset bytes, no pack assets — the catalog parses sources.
    expect(definitions).toHaveBeenCalledExactlyOnceWith(JSON.stringify(files), JSON.stringify({
      base_files: { 'assets/factions/alliance.toml': 'name = "Alliance"\n' },
      packs: [{ id: 'raiders', manifest_toml: '[pack]\n', files: { 'assets/factions/pirate.toml': 'name = "Pirate"\n' } }] }));
    const request = { document_path: 'assets/factions/mine.toml', expected_source: files['assets/factions/mine.toml'],
      edits: [{ op: 'put', path: ['enemies'], value_source: '["b"]' }] };
    expect(await runtime.edit(files['assets/factions/mine.toml'], request)).toContain('enemies = ["b"]');
    expect(edit).toHaveBeenCalledExactlyOnceWith(files['assets/factions/mine.toml'], JSON.stringify(request));
    expect(await runtime.newFaction('Harrow', 'u')).toContain('name = "Harrow"');
    expect(newFaction).toHaveBeenCalledExactlyOnceWith('Harrow', 'u');
  });
});
