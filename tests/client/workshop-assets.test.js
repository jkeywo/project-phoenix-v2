import { describe, expect, it, vi } from 'vitest';
import { createWorkshopAssetSnapshot } from '../../editor/workshop-assets.js';
import { crc32 } from '../../editor/mod-pack-export.js';

describe('immutable Workshop asset dependencies', () => {
  const path = 'assets/models/probe/texture.png';
  const bytes = new Uint8Array([0, 255, 2, 13, 10]);
  const source = () => ({ base_files: { manifest: 'original' },
    base_asset_manifest: { [path]: { length: bytes.length, crc32: crc32(bytes) } }, packs: [] });
  const response = value => ({ ok: true, arrayBuffer: async () => Uint8Array.from(value).buffer });
  it('captures a changed buffer’s existing model consumers and their other immutable dependencies', async () => {
    const model = 'assets/models/probe/mesh.glb', buffer = 'assets/models/probe/mesh.bin';
    const dependencies = source();
    dependencies.base_asset_manifest[model] = { length: bytes.length, crc32: crc32(bytes), requires: [buffer, path] };
    dependencies.base_asset_manifest[buffer] = { length: bytes.length, crc32: crc32(bytes) };
    const fetch = vi.fn(async () => response(bytes));
    const captured = await createWorkshopAssetSnapshot(dependencies, { fetch }).capture([buffer]);
    expect(Object.keys(captured.base_assets).sort()).toEqual([buffer, model, path].sort());
    expect(fetch.mock.calls.map(([path]) => path).sort()).toEqual([buffer, model, path].sort());
  });
  it('captures base dependencies for an accepted model without refetching that model', async () => {
    const model = 'assets/models/accepted.glb', buffer = 'assets/models/vertices.bin';
    const dependencies = source();
    dependencies.packs.push({ asset_dependencies: { [model]: [buffer, path], [buffer]: [] } });
    const fetch = vi.fn(async () => response(bytes));
    const captured = await createWorkshopAssetSnapshot(dependencies, { fetch }).capture([buffer]);
    expect(Object.keys(captured.base_assets)).toEqual([path]);
    expect(fetch).toHaveBeenCalledExactlyOnceWith(path);
  });
  it('refuses buffer validation if a declared consumer cannot be captured unchanged', async () => {
    const model = 'assets/models/probe/mesh.glb', buffer = 'assets/models/probe/mesh.bin';
    for (const failure of [{ ok: false }, response([0, 0, 0, 0, 0])]) {
      const dependencies = source();
      dependencies.base_asset_manifest[model] = { length: bytes.length, crc32: crc32(bytes), requires: [buffer] };
      const snapshot = createWorkshopAssetSnapshot(dependencies, { fetch: async () => failure });
      await expect(snapshot.capture([buffer])).rejects.toThrow(`Immutable model dependency is unavailable: ${model}`);
    }
  });
  it('fetches declared missing bytes once and isolates source and returned snapshots', async () => {
    const dependencies = source();
    const fetch = vi.fn(async () => response(bytes));
    const reader = createWorkshopAssetSnapshot(dependencies, { fetch });
    dependencies.base_files.manifest = 'changed';
    const first = await reader.capture([path, path]);
    expect(first.base_files.manifest).toBe('original');
    expect(first.base_assets[path]).toEqual(Array.from(bytes));
    first.base_assets[path][0] = 50;
    first.base_files.manifest = 'mutated return value';
    first.base_asset_manifest[path].crc32 = 0;
    first.packs.push({ assets: { other: [1] } });
    const next = await reader.capture([path]);
    expect(next.base_assets[path][0]).toBe(0);
    expect(next.base_files.manifest).toBe('original');
    expect(next.base_asset_manifest[path].crc32).toBe(crc32(bytes));
    expect(next.packs).toEqual([]);
    expect(fetch).toHaveBeenCalledExactlyOnceWith(path);
  });
  it('never fetches supplied pack bytes, undeclared paths or external/traversal paths', async () => {
    const dependencies = source();
    dependencies.packs.push({ assets: { [path]: [255, 2] } });
    const fetch = vi.fn();
    const reader = createWorkshopAssetSnapshot(dependencies, { fetch });
    await reader.capture([path, 'assets/missing.png', 'https://example.invalid/image.png', 'assets/../private']);
    expect(fetch).not.toHaveBeenCalled();
  });
  it('refuses changed bytes and permits retry against the same declared identity', async () => {
    const fetch = vi.fn().mockResolvedValueOnce(response([1, 1, 1, 1, 1])).mockResolvedValueOnce(response(bytes));
    const reader = createWorkshopAssetSnapshot(source(), { fetch });
    expect((await reader.capture([path])).base_assets[path]).toBeUndefined();
    expect((await reader.capture([path])).base_assets[path]).toEqual(Array.from(bytes));
    expect(fetch).toHaveBeenCalledTimes(2);
  });
  it('cancels an oversized delivery before retaining bytes beyond the declared length', async () => {
    const cancel = vi.fn();
    const releaseLock = vi.fn();
    const reader = { read: vi.fn().mockResolvedValue({ value: new Uint8Array(6), done: false }), cancel, releaseLock };
    const fetch = vi.fn(async () => ({ ok: true, body: { getReader: () => reader } }));
    const snapshot = await createWorkshopAssetSnapshot(source(), { fetch }).capture([path]);
    expect(snapshot.base_assets[path]).toBeUndefined();
    expect(cancel).toHaveBeenCalledOnce();
    expect(releaseLock).toHaveBeenCalledOnce();
    expect(reader.read).toHaveBeenCalledOnce();
  });
});
