import { describe, it, expect, vi, afterEach } from 'vitest';
import { readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';

afterEach(() => { vi.unstubAllGlobals(); vi.useRealTimers(); vi.resetModules(); });

describe('planet UASTC worker lifecycle', () => {
  async function setup() {
    const workers = [];
    vi.stubGlobal('Worker', class {
      constructor(url) { this.url = url; workers.push(this); }
      postMessage = vi.fn();
      terminate = vi.fn();
    });
    const { transcode } = await import('../../assets/texture-codecs/uastc.js');
    return { workers, transcode };
  }
  it('serializes requests and releases each worker after success or failure', async () => {
    const { workers, transcode } = await setup();
    const first = transcode(new Uint8Array(1), 'bc7');
    const second = transcode(new Uint8Array(1), 'rgba');
    await vi.waitFor(() => expect(workers).toHaveLength(1));
    workers[0].onmessage({ data: { buffer: new Uint8Array([7]).buffer } });
    expect(await first).toEqual(new Uint8Array([7]));
    await vi.waitFor(() => expect(workers).toHaveLength(2));
    const rejected = expect(second).rejects.toThrow('bad input');
    workers[1].onmessage({ data: { error: 'bad input' } });
    await rejected;
    expect(workers.every(worker => worker.terminate.mock.calls.length === 1)).toBe(true);
    expect(workers[0].url.pathname).toContain('/assets/texture-codecs/uastc-worker.js');
  });
  it('times out stalled decoding so Bevy can load the original', async () => {
    vi.useFakeTimers();
    const { workers, transcode } = await setup();
    const pending = transcode(new Uint8Array(1), 'rgba');
    const rejected = expect(pending).rejects.toThrow('timed out');
    await vi.advanceTimersByTimeAsync(30001);
    await rejected;
    expect(workers[0].terminate).toHaveBeenCalledOnce();
  });
});

it('ships the baked source and pinned transcoder together', () => {
  const manifest = JSON.parse(readFileSync('assets/texture-codecs/manifest.json'));
  for (const [file, hash] of Object.entries(manifest.hashes)) {
    expect(createHash('sha256').update(readFileSync(file)).digest('hex'), file).toBe(hash);
  }
});
