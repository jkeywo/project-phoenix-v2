import { describe, expect, it, vi } from 'vitest';
import { createWorkshopAssetSnapshot } from '../workshop-assets.js';
import { crc32 } from '../mod-pack-export.js';

const first = 'assets/models/first.bin', second = 'assets/shaders/test.wgsl';
const a = Uint8Array.of(0, 255, 13, 10), b = new TextEncoder().encode('// captured support\r\n');
const manifest = bytes => ({ length: bytes.length, crc32: crc32(bytes), requires: [] });
function fixture() {
  const source = { base_files: { 'assets/scenarios.toml': '# exact\r\n' },
    base_asset_manifest: { [first]: manifest(a), [second]: manifest(b) }, packs: [] };
  const fetch = vi.fn(async path => ({ ok: true, arrayBuffer: async () => (path === first ? a : b).slice().buffer }));
  return { source, fetch, snapshot: createWorkshopAssetSnapshot(source, { fetch }) };
}

describe('compact immutable browser Test dependencies', () => {
  it('captures the complete declared Test source as independent typed buffers exactly once', async () => {
    const { source, fetch, snapshot } = fixture();
    source.base_files['assets/scenarios.toml'] = 'later delivery';
    const captured = await snapshot.captureAllBuffers();
    expect(captured.base_files['assets/scenarios.toml']).toBe('# exact\r\n');
    expect(captured.base_assets[first]).toBeInstanceOf(Uint8Array);
    expect(captured.base_assets[first]).toEqual(a);
    expect(captured.base_assets[second]).toEqual(b);
    captured.base_assets[first].fill(0);
    expect((await snapshot.captureAllBuffers()).base_assets[first]).toEqual(a);
    expect(fetch).toHaveBeenCalledTimes(2);
  });

  it('keeps later validator JSON restricted to its requested closure after full Test capture', async () => {
    const { snapshot, fetch } = fixture();
    await snapshot.captureAllBuffers();
    const validation = await snapshot.capture([first]);
    expect(validation.base_assets).toEqual({ [first]: Array.from(a) });
    expect(JSON.parse(JSON.stringify(validation)).base_assets).toEqual({ [first]: Array.from(a) });
    expect((await snapshot.capture([])).base_assets).toEqual({});
    const compact = await snapshot.captureBuffers([second]);
    expect(compact.base_assets).toEqual({ [second]: b });
    expect(fetch).toHaveBeenCalledTimes(2);
  });

  it('refuses a declared Test member that changed, failed, or exceeds the capture bound', async () => {
    for (const outcome of ['missing', 'changed', 'oversized']) {
      const source = { base_files: {}, packs: [], base_asset_manifest: {
        [first]: outcome === 'oversized' ? { length: 512 * 1024 * 1024 + 1, crc32: 0 } : manifest(a),
      } };
      const fetch = vi.fn(async () => ({ ok: outcome !== 'missing', arrayBuffer: async () => Uint8Array.of(1, 2, 3, 4).buffer }));
      const snapshot = createWorkshopAssetSnapshot(source, { fetch });
      await expect(snapshot.captureAllBuffers()).rejects.toThrow(first);
      if (outcome === 'oversized') expect(fetch).not.toHaveBeenCalled();
    }
  });

  it('retains accepted dependency bytes and ordering without allowing caller mutations', async () => {
    const bytes = a.slice();
    const source = { base_files: {}, packs: [{ id: 'one', files: {}, assets: { [first]: bytes } }] };
    const fetch = vi.fn();
    const snapshot = createWorkshopAssetSnapshot(source, { fetch });
    bytes.fill(9);
    const captured = await snapshot.captureAllBuffers();
    expect(captured.packs[0].assets[first]).toEqual(a);
    expect(captured.packs[0].assets[first]).toBeInstanceOf(Uint8Array);
    captured.packs[0].assets[first].fill(7);
    expect((await snapshot.captureAllBuffers()).packs[0].assets[first]).toEqual(a);
    expect(fetch).not.toHaveBeenCalled();
  });

  it('refuses metadata-only live pack references in a fresh Test with no asset owner', async () => {
    const fetch = vi.fn();
    const snapshot = createWorkshopAssetSnapshot({ base_files: {}, packs: [
      { id: 'live-only', files: {}, asset_dependencies: { [first]: [] } },
    ] }, { fetch });
    // Ordinary host preflight owns these accepted bytes in its runtime.
    expect((await snapshot.capture([first])).base_assets).toEqual({});
    await expect(snapshot.captureAllBuffers()).rejects.toThrow(first);
    expect(fetch).not.toHaveBeenCalled();
  });

  it('rejects invalid supplied bytes instead of silently truncating them', () => {
    for (const bytes of [[256], [-1], [1.5], { 0: 1 }]) {
      expect(() => createWorkshopAssetSnapshot({ base_files: {}, base_assets: { [first]: bytes } })).toThrow(first);
    }
  });
});
