import { describe, expect, it } from 'vitest';
import { createStoreZip, readStoreZipArchive, isPackAssetPath } from '../mod-pack-export.js';

describe('Workshop asset archive envelope', () => {
  const entry = { path: 'assets/models/part/data.bin', bytes: new Uint8Array([0, 255, 13, 10, 128]) };
  it('keeps exact binary bytes and refuses duplicate members before choosing a winner', () => {
    const zip = createStoreZip([entry]);
    expect(readStoreZipArchive(zip, { binary: isPackAssetPath }).source.entries[0].bytes).toEqual(entry.bytes);
    expect(() => readStoreZipArchive(createStoreZip([entry, entry]), { binary: isPackAssetPath })).toThrow(/duplicate/);
  });
  it('refuses unsupported local flags and inconsistent central flags or disk', () => {
    const zip = createStoreZip([entry]);
    const central = 30 + new TextEncoder().encode(entry.path).length + entry.bytes.length;
    for (const offset of [6, central + 8, central + 34]) {
      const broken = Uint8Array.from(zip);
      broken[offset] = 1;
      expect(() => readStoreZipArchive(broken, { binary: isPackAssetPath })).toThrow();
    }
  });
  it('only admits portable local paths for runtime assets', () => {
    expect(isPackAssetPath(entry.path)).toBe(true);
    for (const path of ['assets/sounds/a.bin', 'assets/models/../escape.glb', 'assets/models/C:/part.glb',
      'assets/models/folder./part.glb', 'assets/models/a..b.glb', 'assets/models/part\u0085.glb']) {
      expect(isPackAssetPath(path), path).toBe(false);
    }
  });
});
