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
    const base = { ...dependencies, packs: [{ id: 'workshop-test', files: {} }, { id: 'other', files: { 'assets/worlds/dependency.toml': '[global]\n' } }] };
    const provider = createBrowserWorkshopProvider({ loadedPack: bytes, dependencies: base,
      load: async () => ({ wasm_workshop_validate_pack() {} }) });
    bytes.fill(0); base.packs[1].files['assets/worlds/dependency.toml'] = 'changed elsewhere';
    const draft = await provider.load();
    expect(draft.sourceBytes()).toEqual(original);
    const snapshot = await provider.runtime.dependencies();
    expect(snapshot.packs).toEqual([{ id: 'other', files: { 'assets/worlds/dependency.toml': '[global]\n' } }]);
    snapshot.packs.length = 0;
    expect((await provider.runtime.dependencies()).packs).toHaveLength(1);
    expect(provider.save).toBeUndefined();
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
