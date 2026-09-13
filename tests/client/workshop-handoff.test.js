import { describe, expect, it, vi } from 'vitest';
import { copyWorkshopSource } from '../../editor/workshop-handoff.js';
import { workshopSourceProvider } from '../../editor/workshop-source-provider.js';
import { createWorkshopSourceTransfer } from '../../gui/workshop-source-link.js';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { WORKSHOP_MANIFEST, WORKSHOP_WORLD, WORKSHOP_WORLD_TEXT, workshopPack } from '../fixtures/workshop-pack.js';

const source = () => ({ selectedId: 'workshop-test', archives: [{ id: 'workshop-test', bytes: workshopPack() }],
  base: { base_files: { 'assets/scenarios.toml': '[content]\nid="phoenix-base"\nepoch=1' }, base_asset_manifest: {} } });

describe('retained Workshop source handoff', () => {
  it('copies exact authored bytes and whitelists source fields, without session identity or undo data', () => {
    const original = { ...source(), operator: { id: 'private' }, credentials: 'secret', history: ['live action'] };
    original.base.session = 'running';
    const copied = copyWorkshopSource(original);
    expect(copied).toEqual(source());
    original.archives[0].bytes.fill(0);
    original.base.base_files['assets/scenarios.toml'] = 'changed';
    expect(copied).toEqual(source());
  });
  it('refuses missing selections, duplicate identities and external dependencies', () => {
    expect(() => copyWorkshopSource({ ...source(), selectedId: 'missing' })).toThrow();
    const duplicate = source(); duplicate.archives.push(duplicate.archives[0]);
    expect(() => copyWorkshopSource(duplicate)).toThrow();
    const external = source(); external.base.base_asset_manifest['assets/model.glb'] = {
      length: 1, crc32: 1, requires: ['https://example.test/asset.bin'],
    };
    expect(() => copyWorkshopSource(external)).toThrow();
  });
  it('opens a fresh editable document over immutable other-pack dependencies', async () => {
    const bundle = source();
    bundle.archives.push({ id: 'readonly-pack', bytes: createStoreZip([
      { path: 'scenarios.toml', text: WORKSHOP_MANIFEST.replaceAll('workshop-test', 'readonly-pack') },
      { path: WORKSHOP_WORLD, text: WORKSHOP_WORLD_TEXT },
      { path: 'assets/models/dependency.bin', bytes: new Uint8Array([7, 8, 255]) },
    ]) });
    const provider = workshopSourceProvider(copyWorkshopSource(bundle), {
      load: async () => ({ wasm_workshop_validate_pack: () => '{}' }),
    });
    const draft = await provider.load();
    expect(draft.archive()).toEqual(workshopPack());
    expect(draft.canUndo()).toBe(false);
    expect(draft.paths()).not.toContain('assets/models/dependency.bin');
    draft.edit(WORKSHOP_WORLD, '# authoring edit\n[global]\n');
    expect(bundle.archives[0].bytes).toEqual(workshopPack());
    const bytes = await provider.runtime.readAsset('assets/models/dependency.bin');
    expect(bytes).toEqual(new Uint8Array([7, 8, 255]));
    bytes.fill(0);
    expect(await provider.runtime.readAsset('assets/models/dependency.bin')).toEqual(new Uint8Array([7, 8, 255]));
  });
});

function controller(overrides = {}) {
  const order = [];
  const capabilities = {
    read: vi.fn(async () => { order.push('read'); return source(); }),
    store: { save: vi.fn(async () => { order.push('store'); return 'retained-source-token'; }), clear: vi.fn(async () => {}) },
    operator: () => ({ id: 'gm-a' }),
    disconnect: vi.fn(() => { order.push('disconnect'); return true; }),
    navigate: vi.fn(url => { order.push('navigate'); expect(url).toBe('workshop.html#source=retained-source-token'); }),
    ...overrides,
  };
  return { capabilities, order, transfer: createWorkshopSourceTransfer(capabilities) };
}
describe('Live to Authoring boundary', () => {
  it('retains the source then disconnects before opening an editable page', async () => {
    const { transfer, order } = controller();
    expect(await transfer.open('workshop-test')).toBe(true);
    expect(order).toEqual(['read', 'store', 'disconnect', 'navigate']);
  });
  it('keeps Live when source retention fails', async () => {
    const { transfer, capabilities } = controller({ store: { save: async () => { throw Error('quota'); }, clear: vi.fn() } });
    await expect(transfer.open('workshop-test')).rejects.toThrow('quota');
    expect(capabilities.disconnect).not.toHaveBeenCalled();
    expect(capabilities.navigate).not.toHaveBeenCalled();
  });
  it('refuses a changed GM identity during async retention and removes its transfer', async () => {
    let owner = 'gm-a';
    const clear = vi.fn(async () => {});
    const { transfer, capabilities } = controller({ operator: () => ({ id: owner }),
      store: { save: async () => { owner = 'gm-b'; return 'retained-source-token'; }, clear } });
    await expect(transfer.open('workshop-test')).rejects.toThrow('GM connection changed');
    expect(clear).toHaveBeenCalledWith('retained-source-token');
    expect(capabilities.disconnect).not.toHaveBeenCalled();
  });
  it('never opens Authoring if ordinary disconnect is refused', async () => {
    const { transfer, capabilities } = controller({ disconnect: () => false });
    await expect(transfer.open('workshop-test')).rejects.toThrow('Could not leave');
    expect(capabilities.store.clear).toHaveBeenCalledWith('retained-source-token');
    expect(capabilities.navigate).not.toHaveBeenCalled();
  });
  it('coalesces repeat activation and cancels departure when disposed during preparation', async () => {
    let finish;
    const read = new Promise(resolve => { finish = resolve; });
    const { transfer, capabilities } = controller({ read: () => read });
    const pending = transfer.open('workshop-test');
    expect(await transfer.open('workshop-test')).toBe(false);
    transfer.dispose(); finish(source());
    await expect(pending).rejects.toThrow('GM connection changed');
    expect(capabilities.disconnect).not.toHaveBeenCalled();
    expect(capabilities.store.save).not.toHaveBeenCalled();
  });
});
