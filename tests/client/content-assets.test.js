import { describe, it, expect, vi } from 'vitest';
import { fetchContentAsset } from '../../gui/content-assets.js';

describe('accepted pack asset delivery', () => {
  it('passes exact non-UTF8 bytes to audio decoding without fetching the base path', async () => {
    const source = new Uint8Array([0, 255, 254, 13, 10]);
    const fetch = vi.fn();
    const read = vi.fn(() => source);
    const response = await fetchContentAsset('assets/sounds/private.ogg', { read, fetch });
    source.fill(1);
    expect(new Uint8Array(await response.arrayBuffer())).toEqual(new Uint8Array([0, 255, 254, 13, 10]));
    expect(read).toHaveBeenCalledWith('assets/sounds/private.ogg');
    expect(fetch).not.toHaveBeenCalled();
  });
  it('uses the same delivery path for base content or a native embedded page', async () => {
    const delivered = { ok: true, arrayBuffer: vi.fn() };
    const fetch = vi.fn(async () => delivered);
    expect(await fetchContentAsset('assets/sounds/base.mp3', { read: () => undefined, fetch })).toBe(delivered);
    expect(await fetchContentAsset('assets/sounds/base.mp3', { read: null, fetch })).toBe(delivered);
    expect(fetch.mock.calls).toEqual([['assets/sounds/base.mp3'], ['assets/sounds/base.mp3']]);
  });
});
