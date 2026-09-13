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

  it('saves native exact source through the private provider and advances only acknowledged revisions', async () => {
    const source = new WorkshopDocument(workshopPack());
    const request = vi.fn(async value => {
      if (value.op === 'load') return { status: 'loaded', kind: 'mod', revision: 'original', files: source.toFiles() };
      if (value.op === 'save') return { status: 'saved', revision: 'saved' };
      if (value.op === 'validate') return { status: 'validated', report: { accepted: true, findings: [] } };
      return { status: 'done' };
    });
    const provider = createNativeWorkshopProvider({ request });
    const draft = await provider.load();
    draft.edit(WORKSHOP_WORLD, '# native edit\n[global]\n');
    await provider.runtime.validate(null, draft);
    await provider.save(draft);
    expect(request).toHaveBeenCalledWith({ op: 'save', files: draft.toFiles(), expected_revision: 'original' });
    await provider.recovery.save({ version: 1, draft: draft.snapshot() });
    const stored = request.mock.calls.at(-1)[0];
    expect(stored.expected_revision).toBe('saved');
    expect(JSON.parse(stored.record).draft.source).toEqual(Array.from(draft.sourceBytes()));
    expect(provider.canImport).toBe(false);
  });

  it('retains recovered old revisions so stale draft saves reach the native conflict gate', async () => {
    const draft = new WorkshopDocument(workshopPack());
    const saved = { version: 1, selected: WORKSHOP_WORLD, draft: draft.snapshot() };
    const request = vi.fn(async value => {
      if (value.op === 'load') return { status: 'loaded', kind: 'mod', revision: 'disk-newer', files: draft.toFiles() };
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
});
