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

  it('reads the composition catalog with text-only dependencies and routes compose requests and new worlds to the runtime (issue #1475)', async () => {
    const source = { base_files: { 'assets/worlds/base.toml': '[global]\n' },
      base_asset_manifest: { 'assets/models/a.glb': { length: 2, crc32: 0 } },
      packs: [{ id: 'raiders', manifest_toml: '[pack]\n', files: { 'assets/worlds/raid.toml': '[global]\n' },
        assets: { 'assets/sounds/x.ogg': [1, 2] } }] };
    const catalog = { manifest: null, worlds: [], members: [], choices: { worlds: [], templates: [] }, catalogue: [], findings: [] };
    const composition = vi.fn(() => JSON.stringify(catalog));
    const compose = vi.fn(() => '[global]\nextra_worlds = ["assets/worlds/base.toml"]\n');
    const newWorld = vi.fn(() => '[global]\ntitle = "Harrow"\n');
    const runtime = createWorkshopRuntime({ load: async () => ({
      wasm_workshop_validate_pack: () => '{}', wasm_workshop_composition: composition, wasm_workshop_compose: compose,
      wasm_workshop_new_world: newWorld,
    }), dependencies: async () => source });
    const files = { 'assets/worlds/mine.toml': '[global]\n' };
    const textOnly = JSON.stringify({ base_files: { 'assets/worlds/base.toml': '[global]\n' },
      packs: [{ id: 'raiders', manifest_toml: '[pack]\n', files: { 'assets/worlds/raid.toml': '[global]\n' } }] });
    expect(await runtime.composition(files)).toEqual(catalog);
    // Text only: no base asset bytes, no pack assets — the catalog parses sources.
    expect(composition).toHaveBeenCalledExactlyOnceWith(JSON.stringify(files), textOnly);
    const request = { document_path: 'assets/worlds/mine.toml', expected_source: '[global]\n',
      edits: [{ op: 'put', path: ['extra_worlds'], value_source: '["assets/worlds/base.toml"]' }] };
    expect(await runtime.compose(files, request)).toContain('extra_worlds = ["assets/worlds/base.toml"]');
    expect(compose).toHaveBeenCalledExactlyOnceWith(JSON.stringify(files), textOnly, JSON.stringify(request));
    expect(await runtime.newWorld('Harrow')).toContain('title = "Harrow"');
    expect(newWorld).toHaveBeenCalledExactlyOnceWith('Harrow');
  });

  it('reads one template\'s composition with text-only dependencies and routes entity edits and materialising (issue #1476)', async () => {
    const source = { base_files: { 'assets/entities/base.toml': 'name = "base"\n' },
      base_asset_manifest: { 'assets/models/a.glb': { length: 2, crc32: 0 } },
      packs: [{ id: 'raiders', manifest_toml: '[pack]\n', files: { 'assets/entities/raid.toml': 'name = "raid"\n' },
        assets: { 'assets/sounds/x.ogg': [1, 2] } }] };
    const composition = { path: 'assets/entities/mine.toml', origin: 'draft', resolvable: true, error: null,
      includes: [], sources: ['assets/entities/mine.toml'], components: [], fields: [], supported_components: ['hull'],
      fragment_choices: [], findings: [] };
    const entity = vi.fn(() => JSON.stringify(composition));
    const entityEdit = vi.fn(() => 'name = "mine"\nincludes = ["fragment.toml"]\n');
    const materialise = vi.fn(() => 'name = "mine"\n[hull]\nhull_integrity = 100\n');
    const runtime = createWorkshopRuntime({ load: async () => ({
      wasm_workshop_validate_pack: () => '{}', wasm_workshop_entity: entity, wasm_workshop_entity_edit: entityEdit,
      wasm_workshop_entity_materialise: materialise,
    }), dependencies: async () => source });
    const files = { 'assets/entities/mine.toml': 'name = "mine"\n' };
    const textOnly = JSON.stringify({ base_files: { 'assets/entities/base.toml': 'name = "base"\n' },
      packs: [{ id: 'raiders', manifest_toml: '[pack]\n', files: { 'assets/entities/raid.toml': 'name = "raid"\n' } }] });
    expect(await runtime.entity(files, 'assets/entities/mine.toml')).toEqual(composition);
    // Text only: no base asset bytes, no pack assets — the resolver parses sources.
    expect(entity).toHaveBeenCalledExactlyOnceWith(JSON.stringify(files), textOnly, 'assets/entities/mine.toml');
    const request = { document_path: 'assets/entities/mine.toml', expected_source: 'name = "mine"\n',
      edits: [{ op: 'put', path: ['includes'], value_source: '["fragment.toml"]' }] };
    expect(await runtime.editEntity(files, request)).toContain('includes = ["fragment.toml"]');
    expect(entityEdit).toHaveBeenCalledExactlyOnceWith(JSON.stringify(files), textOnly, JSON.stringify(request));
    expect(await runtime.materialiseEntity(files, 'assets/entities/mine.toml', 'hull.hull_integrity'))
      .toContain('hull_integrity = 100');
    expect(materialise).toHaveBeenCalledExactlyOnceWith(JSON.stringify(files), textOnly, 'assets/entities/mine.toml',
      'hull.hull_integrity');
  });

  it('reads one world\'s GM role presets with text-only dependencies and routes preset edits and new presets (issue #1477)', async () => {
    const source = { base_files: { 'assets/worlds/base.toml': '[global]\n' },
      base_asset_manifest: { 'assets/models/a.glb': { length: 2, crc32: 0 } },
      packs: [{ id: 'raiders', manifest_toml: '[pack]\n', files: { 'assets/worlds/raid.toml': '[global]\n' },
        assets: { 'assets/sounds/x.ogg': [1, 2] } }] };
    const catalog = { path: 'assets/worlds/mine.toml', origin: 'draft', presets: [],
      choices: { widget_types: ['attention'], widget_actions: [], bands: [], categories: [], entities: [] },
      worlds: [], findings: [] };
    const presets = vi.fn(() => JSON.stringify(catalog));
    const presetsEdit = vi.fn(() => '[[gm_role_preset]]\nid = "watch"\nlabel = "server.gm.watch"\n');
    const newPreset = vi.fn(() => '[[gm_role_preset]]\nid = "watch"\nlabel = "server.gm.watch"\n');
    const runtime = createWorkshopRuntime({ load: async () => ({
      wasm_workshop_validate_pack: () => '{}', wasm_workshop_presets: presets,
      wasm_workshop_presets_edit: presetsEdit, wasm_workshop_new_preset: newPreset,
    }), dependencies: async () => source });
    const files = { 'assets/worlds/mine.toml': '[global]\n' };
    const textOnly = JSON.stringify({ base_files: { 'assets/worlds/base.toml': '[global]\n' },
      packs: [{ id: 'raiders', manifest_toml: '[pack]\n', files: { 'assets/worlds/raid.toml': '[global]\n' } }] });
    expect(await runtime.presets(files, 'assets/worlds/mine.toml')).toEqual(catalog);
    // Text only: no base asset bytes, no pack assets — the reading parses sources.
    expect(presets).toHaveBeenCalledExactlyOnceWith(JSON.stringify(files), textOnly, 'assets/worlds/mine.toml');
    const request = { document_path: 'assets/worlds/mine.toml', expected_source: '[global]\n',
      edits: [{ op: 'set', path: ['gm_role_preset', 0, 'label'], value_source: '"server.gm.watch"' }] };
    expect(await runtime.editPresets(files, request)).toContain('id = "watch"');
    expect(presetsEdit).toHaveBeenCalledExactlyOnceWith(JSON.stringify(files), textOnly, JSON.stringify(request));
    expect(await runtime.newPreset('watch', 'server.gm.watch')).toContain('[[gm_role_preset]]');
    expect(newPreset).toHaveBeenCalledExactlyOnceWith('watch', 'server.gm.watch');
  });

  it('reads Rhai completion and diagnostics from the runtime authoring registry', async () => {
    const registry = [{ name: 'on_timer', receiver: '', category: 'register' }];
    const diagnostics = [{ severity: 'error', message: 'Expected expression', line: 9, column: 2 }];
    const hostFns = vi.fn(() => registry), compile = vi.fn(() => diagnostics);
    const runtime = createWorkshopRuntime({ load: async () => ({
      wasm_workshop_validate_pack: () => '{}',
      wasm_get_script_host_fns: hostFns,
      wasm_script_diagnostics: compile,
    }), dependencies: async () => ({ base_files: {} }) });
    expect(await runtime.scriptHostFunctions()).toEqual(registry);
    expect(await runtime.scriptDiagnostics('fn broken( {', 8)).toEqual(diagnostics);
    expect(compile).toHaveBeenCalledWith('fn broken( {', 8);
  });
});
