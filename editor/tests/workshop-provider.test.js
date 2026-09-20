import { describe, it, expect, vi } from 'vitest';
import { createNativeWorkshopProvider, createBrowserWorkshopProvider, createWorkshopBridge, newWorkshopPack } from '../workshop-provider.js';
import { WorkshopDocument } from '../workshop-document.js';
import { workshopPack, WORKSHOP_WORLD } from '../../tests/fixtures/workshop-pack.js';

const dependencies = { base_files: { 'assets/scenarios.toml': '[content]\nid="phoenix-base"\nepoch=1\n' }, packs: [] };
describe('explicit Workshop capability providers', () => {
  it('creates one complete runtime-shaped pack and clones only the selected loaded source', async () => {
    expect(newWorkshopPack(dependencies).check().ok).toBe(true);
    const bytes = workshopPack();
    const original = Uint8Array.from(bytes);
    const manifest = '# exact dependency\r\n[pack]\r\nid="other"\r\n';
    const base = { ...dependencies, packs: [{ id: 'workshop-test', manifest_toml: 'selected', files: {} },
      { id: 'other', manifest_toml: manifest, files: { 'assets/worlds/dependency.toml': '[global]\n' } }] };
    const provider = createBrowserWorkshopProvider({ loadedPack: bytes, dependencies: base,
      load: async () => ({ wasm_workshop_validate_pack() {} }) });
    bytes.fill(0); base.packs[1].files['assets/worlds/dependency.toml'] = 'changed elsewhere';
    const draft = await provider.load();
    expect(draft.sourceBytes()).toEqual(original);
    const snapshot = await provider.runtime.dependencies();
    expect(snapshot.packs).toEqual([{ id: 'other', manifest_toml: manifest,
      files: { 'assets/worlds/dependency.toml': '[global]\n' } }]);
    snapshot.packs.length = 0;
    expect((await provider.runtime.dependencies()).packs).toHaveLength(1);
    expect(provider.save).toBeUndefined();
    expect(provider.billboardCapture).toBeUndefined();
  });

  it('keeps supplied binary dependencies compact and immutable through the browser provider', async () => {
    const path = 'assets/models/fixture.bin', bytes = Uint8Array.of(0, 255, 13, 10);
    const provider = createBrowserWorkshopProvider({ dependencies: { ...dependencies,
      packs: [{ id: 'read-only', files: {}, assets: { [path]: bytes } }] },
    load: async () => ({ wasm_workshop_validate_pack() {} }) });
    bytes.fill(8);
    expect(await provider.runtime.readAsset(path)).toEqual(Uint8Array.of(0, 255, 13, 10));
    const snapshot = await provider.runtime.testDependencies();
    expect(snapshot.packs[0].assets[path]).toBeInstanceOf(Uint8Array);
    snapshot.packs[0].assets[path].fill(9);
    expect(await provider.runtime.readAsset(path)).toEqual(Uint8Array.of(0, 255, 13, 10));
  });

  it('saves native exact source through the private provider and advances only acknowledged revisions', async () => {
    const source = new WorkshopDocument(workshopPack());
    const request = vi.fn(async value => {
      if (value.op === 'load-sources') return { status: 'sources', kind: 'mod', revision: 'original', files: source.toFiles() };
      if (value.op === 'save-sources') return { status: 'saved', revision: 'saved' };
      if (value.op === 'validate-sources') return { status: 'validated', report: { accepted: true, findings: [] } };
      return { status: 'done' };
    });
    const provider = createNativeWorkshopProvider({ request });
    const draft = await provider.load();
    draft.edit(WORKSHOP_WORLD, '# native edit\n[global]\n');
    await provider.runtime.validate(null, draft);
    await provider.save(draft);
    expect(request).toHaveBeenCalledWith({ op: 'save-sources', files: draft.toNativeSources(), expected_revision: 'original' });
    await provider.recovery.save({ version: 1, draft: draft.snapshot() });
    const stored = request.mock.calls.at(-1)[0];
    expect(stored.expected_revision).toBe('saved');
    expect(JSON.parse(stored.record).draft.sourceFiles).toEqual(draft.snapshot().sourceFiles);
    expect(provider.canImport).toBe(false);
    expect(provider.billboardCapture).toBeDefined();
  });

  it('reports an invalid native dependency response with a presenter-localizable code', async () => {
    for (const response of [{ status: 'done' },
      { status: 'dependencies', base_files: {}, packs: [{ id: 'other', files: {} }] },
      { status: 'dependencies', base_files: {}, packs: [{ id: 'other', manifest_toml: 'exact', files: { bad: 7 } }] }]) {
      const provider = createNativeWorkshopProvider({ request: async () => response });
      await expect(provider.runtime.dependencies()).rejects.toMatchObject({
        code: 'native-workshop-dependencies-invalid',
        message: '',
      });
    }
  });

  it('preserves the exact dependency manifest across the native provider boundary', async () => {
    const manifest_toml = '# exact dependency\r\n[pack]\r\nid="other"\r\n';
    const provider = createNativeWorkshopProvider({ request: async value => value.op === 'load-dependencies'
      ? { status: 'dependencies', base_files: {}, packs: [{ id: 'other', manifest_toml, files: {} }] }
      : { status: 'done' } });
    expect((await provider.runtime.dependencies()).packs[0]).toEqual({ id: 'other', manifest_toml, files: {} });
  });

  it('routes Rhai authoring through the private native provider', async () => {
    const functions = [{ name: 'on_timer', receiver: '', category: 'register' }];
    const diagnostics = [{ severity: 'error', message: 'Expected expression', line: 9, column: 2 }];
    const request = vi.fn(async value => value.op === 'script-host-functions'
      ? { status: 'script-host-functions', functions }
      : { status: 'script-diagnostics', diagnostics });
    const provider = createNativeWorkshopProvider({ request });
    expect(await provider.runtime.scriptHostFunctions()).toEqual(functions);
    expect(await provider.runtime.scriptDiagnostics('fn broken( {', 8)).toEqual(diagnostics);
    expect(request.mock.calls.at(-1)[0]).toEqual({ op: 'script-diagnostics', source: 'fn broken( {', line_offset: 8 });
  });

  it('reads native ship authoring choices from the runtime-owned schema endpoint', async () => {
    const request = vi.fn(async value => value.op === 'ship-schema'
      ? { status: 'ship-schema', schema: { system_kinds: ['helm_thrust'], directive_kinds: ['None', 'Destroy'] } }
      : { status: 'done' });
    const provider = createNativeWorkshopProvider({ request });
    await expect(provider.runtime.shipSchema()).resolves.toEqual({ system_kinds: ['helm_thrust'], directive_kinds: ['None', 'Destroy'] });
    expect(request).toHaveBeenCalledWith({ op: 'ship-schema' });
  });

  it('retains recovered old revisions so stale draft saves reach the native conflict gate', async () => {
    const draft = new WorkshopDocument(workshopPack());
    const saved = { version: 1, selected: WORKSHOP_WORLD, draft: draft.snapshot() };
    const request = vi.fn(async value => {
      if (value.op === 'load-sources') return { status: 'sources', kind: 'mod', revision: 'disk-newer', files: draft.toFiles() };
      if (value.op === 'recovery-load') return { status: 'recovery', recovery: { revision: 'draft-older', record: JSON.stringify(saved, (_k, value) => value instanceof Uint8Array ? Array.from(value) : value) } };
      return { status: 'refused', message: 'External edit', report: null };
    });
    const provider = createNativeWorkshopProvider({ request });
    await provider.load();
    const recovered = await provider.recovery.load();
    expect(WorkshopDocument.restore(recovered.draft).sourceBytes()).toEqual(draft.sourceBytes());
    provider.restore(recovered);
    await expect(provider.save(draft)).rejects.toThrow('External edit');
    expect(request.mock.calls.at(-1)[0].expected_revision).toBe('draft-older');
  });

  it('routes definition readings, structural edits and new factions over the bridge with the native op spellings', async () => {
    const catalog = { factions: [], hulls: [], choices: {}, defaults: {}, findings: [] };
    const request = vi.fn(async value => {
      if (value.op === 'definitions') return { status: 'definitions', catalog };
      if (value.op === 'edit') return { status: 'patched', source: `${value.source}# edited\n` };
      if (value.op === 'new-faction') return { status: 'patched', source: `uuid = "${value.uuid}"\nname = "${value.name}"\n` };
      return { status: 'done' };
    });
    const provider = createNativeWorkshopProvider({ request });
    const files = { 'assets/factions/mine.toml': 'name = "Mine"\n' };
    expect(await provider.runtime.definitions(files)).toEqual(catalog);
    expect(request).toHaveBeenCalledWith({ op: 'definitions', files });
    const edit = { document_path: 'assets/factions/mine.toml', expected_source: 'name = "Mine"\n',
      edits: [{ op: 'set', path: ['name'], value_source: '"Mine Two"' }] };
    expect(await provider.runtime.edit('name = "Mine"\n', edit)).toBe('name = "Mine"\n# edited\n');
    expect(request).toHaveBeenCalledWith({ op: 'edit', source: 'name = "Mine"\n', edit });
    expect(await provider.runtime.newFaction('Harrow', 'u')).toBe('uuid = "u"\nname = "Harrow"\n');
    expect(request).toHaveBeenCalledWith({ op: 'new-faction', name: 'Harrow', uuid: 'u' });
    const broken = createNativeWorkshopProvider({ request: async () => ({ status: 'done' }) });
    await expect(broken.runtime.definitions(files)).rejects.toThrow('Invalid native definition catalog');
    const refused = createNativeWorkshopProvider({ request: async () => ({ status: 'refused', message: 'Stale source', report: null }) });
    await expect(refused.runtime.edit('x', edit)).rejects.toThrow('Stale source');
  });

  it('routes composition readings, compose requests and new worlds over the bridge with the native op spellings (issue #1475)', async () => {
    const catalog = { manifest: null, worlds: [], members: [], choices: {}, catalogue: [], findings: [] };
    const request = vi.fn(async value => {
      if (value.op === 'composition') return { status: 'composition', catalog };
      if (value.op === 'compose') return { status: 'patched', source: `${value.files[value.request.document_path]}# composed\n` };
      if (value.op === 'new-world') return { status: 'patched', source: `[global]\ntitle = "${value.title}"\n` };
      return { status: 'done' };
    });
    const provider = createNativeWorkshopProvider({ request });
    const files = { 'assets/worlds/mine.toml': '[global]\n' };
    expect(await provider.runtime.composition(files)).toEqual(catalog);
    expect(request).toHaveBeenCalledWith({ op: 'composition', files });
    const compose = { document_path: 'assets/worlds/mine.toml', expected_source: '[global]\n',
      edits: [{ op: 'put', path: ['extra_worlds'], value_source: '["assets/worlds/base.toml"]' }] };
    expect(await provider.runtime.compose(files, compose)).toBe('[global]\n# composed\n');
    expect(request).toHaveBeenCalledWith({ op: 'compose', files, request: compose });
    expect(await provider.runtime.newWorld('Harrow')).toBe('[global]\ntitle = "Harrow"\n');
    expect(request).toHaveBeenCalledWith({ op: 'new-world', title: 'Harrow' });
    const broken = createNativeWorkshopProvider({ request: async () => ({ status: 'done' }) });
    await expect(broken.runtime.composition(files)).rejects.toThrow('Invalid native composition catalog');
    // A refusal keeps the runtime's own words: the panel maps them to a category.
    const refused = createNativeWorkshopProvider({ request: async () => ({ status: 'refused',
      message: 'extra_worlds entry assets/worlds/x.toml would form a cycle', report: null }) });
    await expect(refused.runtime.compose(files, compose)).rejects.toThrow('would form a cycle');
  });

  it('routes entity readings, entity edits and materialising over the bridge with the native op spellings (issue #1476)', async () => {
    const composition = { path: 'assets/entities/mine.toml', origin: 'draft', resolvable: true, error: null, includes: [],
      sources: [], components: [], fields: [], supported_components: [], fragment_choices: [], findings: [] };
    const request = vi.fn(async value => {
      if (value.op === 'entity') return { status: 'entity', composition };
      if (value.op === 'entity-edit') return { status: 'patched', source: `${value.files[value.request.document_path]}# edited\n` };
      if (value.op === 'entity-materialise') return { status: 'patched', source: `${value.files[value.path]}# ${value.address}\n` };
      return { status: 'done' };
    });
    const provider = createNativeWorkshopProvider({ request });
    const files = { 'assets/entities/mine.toml': 'name = "mine"\n' };
    expect(await provider.runtime.entity(files, 'assets/entities/mine.toml')).toEqual(composition);
    expect(request).toHaveBeenCalledWith({ op: 'entity', files, path: 'assets/entities/mine.toml' });
    const edit = { document_path: 'assets/entities/mine.toml', expected_source: 'name = "mine"\n',
      edits: [{ op: 'put', path: ['includes'], value_source: '["fragments/ai/base.toml"]' }] };
    expect(await provider.runtime.editEntity(files, edit)).toBe('name = "mine"\n# edited\n');
    expect(request).toHaveBeenCalledWith({ op: 'entity-edit', files, request: edit });
    expect(await provider.runtime.materialiseEntity(files, 'assets/entities/mine.toml', 'hull.hull_integrity'))
      .toBe('name = "mine"\n# hull.hull_integrity\n');
    expect(request).toHaveBeenCalledWith({ op: 'entity-materialise', files, path: 'assets/entities/mine.toml',
      address: 'hull.hull_integrity' });
    const broken = createNativeWorkshopProvider({ request: async () => ({ status: 'done' }) });
    await expect(broken.runtime.entity(files, 'assets/entities/mine.toml')).rejects.toThrow('Invalid native entity composition');
    // A refusal keeps the runtime's own words: the panel maps them to a category.
    const refused = createNativeWorkshopProvider({ request: async () => ({ status: 'refused',
      message: 'include-cycle: fragments/ai/base.toml would form a cycle', report: null }) });
    await expect(refused.runtime.editEntity(files, edit)).rejects.toThrow('include-cycle');
  });

  it('routes preset readings, preset edits and new presets over the bridge with the native op spellings (issue #1477)', async () => {
    const presets = { path: 'assets/worlds/mine.toml', origin: 'draft', presets: [],
      choices: { widget_types: [], widget_actions: [], bands: [], categories: [], entities: [] },
      worlds: [], findings: [] };
    const request = vi.fn(async value => {
      if (value.op === 'presets') return { status: 'presets', presets };
      if (value.op === 'presets-edit') return { status: 'patched', source: `${value.files[value.request.document_path]}# edited\n` };
      if (value.op === 'new-preset') return { status: 'patched', source: `[[gm_role_preset]]\nid = "${value.preset_id}"\nlabel = "${value.label}"\n` };
      return { status: 'done' };
    });
    const provider = createNativeWorkshopProvider({ request });
    const files = { 'assets/worlds/mine.toml': '[global]\n' };
    expect(await provider.runtime.presets(files, 'assets/worlds/mine.toml')).toEqual(presets);
    expect(request).toHaveBeenCalledWith({ op: 'presets', files, path: 'assets/worlds/mine.toml' });
    const edit = { document_path: 'assets/worlds/mine.toml', expected_source: '[global]\n',
      edits: [{ op: 'put', path: ['gm_role_preset', 0, 'panels'], value_source: '["gm-map-panel"]' }] };
    expect(await provider.runtime.editPresets(files, edit)).toBe('[global]\n# edited\n');
    expect(request).toHaveBeenCalledWith({ op: 'presets-edit', files, request: edit });
    expect(await provider.runtime.newPreset('watch', 'server.gm.watch')).toContain('id = "watch"');
    expect(request).toHaveBeenCalledWith({ op: 'new-preset', preset_id: 'watch', label: 'server.gm.watch' });
    // `preset_id`, never `id`: the bridge spreads the operation OVER its own
    // envelope (`{ id, ...operation }`), so an operation carrying `id` would
    // replace the correlation number the host reads the reply back by — and the
    // host removes that field before the typed operation is read at all. No
    // operation may name it, which is why a preset's own id travels as
    // `preset_id`.
    for (const [value] of request.mock.calls) expect(value).not.toHaveProperty('id');
    const sent = [];
    const bridge = createWorkshopBridge({ send: value => sent.push(JSON.parse(value)) });
    const pending = bridge.request(request.mock.calls.at(-1)[0]).catch(() => {});
    expect(sent[0]).toEqual({ id: 1, op: 'new-preset', preset_id: 'watch', label: 'server.gm.watch' });
    bridge.dispose();
    await pending;
    const broken = createNativeWorkshopProvider({ request: async () => ({ status: 'done' }) });
    await expect(broken.runtime.presets(files, 'assets/worlds/mine.toml')).rejects.toThrow('Invalid native preset catalog');
    // A refusal keeps the runtime's own words: the panel maps them to a category.
    const refused = createNativeWorkshopProvider({ request: async () => ({ status: 'refused',
      message: 'widget-unknown-band: unknown attention band "puce"', report: null }) });
    await expect(refused.runtime.editPresets(files, edit)).rejects.toThrow('widget-unknown-band');
  });

  it('correlates private replies and rejects pending work when its surface closes', async () => {
    const sent = [];
    const bridge = createWorkshopBridge({ send: value => sent.push(JSON.parse(value)) });
    const first = bridge.request({ op: 'load' });
    const second = bridge.request({ op: 'recovery-load' });
    expect(bridge.receive({ id: 987, status: 'done' })).toBe(false);
    bridge.receive({ id: sent[0].id, status: 'loaded' });
    await expect(first).resolves.toMatchObject({ status: 'loaded' });
    bridge.dispose();
    await expect(second).rejects.toThrow('surface closed');
  });

  it('transfers native assets in bounded chunks and leaves recovery/history compact', async () => {
    const bytes = Uint8Array.from({ length: 150000 }, (_, index) => index % 251);
    const reference = { asset: `0000000000000001-${bytes.length}`, length: bytes.length };
    const source = new WorkshopDocument(workshopPack());
    const received = [];
    const request = vi.fn(async value => {
      if (value.op === 'load-sources') return { status: 'sources', kind: 'mod', revision: 'original', files: source.toFiles() };
      if (value.op === 'asset-begin') return { status: 'asset-upload', token: 'one' };
      if (value.op === 'asset-chunk') { expect(value.offset).toBe(received.length); received.push(...value.bytes); return { status: 'done' }; }
      if (value.op === 'asset-finish') return { status: 'asset-stored', reference };
      if (value.op === 'asset-read') return { status: 'asset-chunk', bytes: Array.from(bytes.slice(value.offset, value.offset + 65536)) };
      return { status: 'done' };
    });
    const provider = createNativeWorkshopProvider({ request });
    const draft = await provider.load();
    const imported = await provider.importAsset(new Blob([bytes]));
    draft.put('assets/sounds/test.mp3', imported);
    expect(received).toEqual(Array.from(bytes));
    expect(request.mock.calls.filter(([value]) => value.op === 'asset-chunk').map(([value]) => value.bytes.length)).toEqual([65536, 65536, 18928]);
    expect(await provider.readAsset(imported)).toEqual(bytes);
    expect(JSON.stringify(draft.snapshot()).length).toBeLessThan(20000);
    const recovered = provider.restoreDocument(draft.snapshot());
    expect(recovered.toFiles()['assets/sounds/test.mp3']).toEqual(reference);
    recovered.undo(); expect(recovered.paths()).not.toContain('assets/sounds/test.mp3');
    recovered.redo(); expect(recovered.toFiles()['assets/sounds/test.mp3']).toEqual(reference);
  });

  it('cancels refused imports and rejects empty or oversized reads instead of hanging', async () => {
    const request = vi.fn(async value => value.op === 'asset-begin' ? { status: 'asset-upload', token: 'one' }
      : value.op === 'asset-cancel' ? { status: 'done' }
        : { status: 'refused', message: 'Storage unavailable' });
    const provider = createNativeWorkshopProvider({ request });
    await expect(provider.importAsset(new Blob(['abc']))).rejects.toThrow('Storage unavailable');
    expect(request.mock.calls.at(-1)[0]).toEqual({ op: 'asset-cancel', token: 'one' });
    const broken = createNativeWorkshopProvider({ request: async () => ({ status: 'asset-chunk', bytes: [] }) });
    await expect(broken.readAsset({ asset: '0000000000000001-3', length: 3 })).rejects.toThrow('Invalid native asset chunk');
  });
});
