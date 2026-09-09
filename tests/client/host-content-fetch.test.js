import { describe, it, expect, vi } from 'vitest';
import { registerContentFetch } from '../../gui/host-content-fetch.js';

function rig(fetcher) {
  let callback;
  const bindings = {
    set_world_fetch_callback: fn => { callback = fn; },
    wasm_push_sidecar_toml: vi.fn(),
    wasm_push_world_toml: vi.fn(),
    wasm_fail_world_fetch: vi.fn(),
  };
  const settled = vi.fn();
  registerContentFetch(bindings, settled, fetcher);
  return { bindings, settled, request: (...args) => callback(...args) };
}

describe('pre-init and runtime content delivery', () => {
  it('waits for the body, delivers empty scripts as content, then checks completion', async () => {
    let release;
    const body = new Promise(resolve => { release = resolve; });
    const fetcher = vi.fn(async () => ({ ok: true, text: () => body }));
    const r = rig(fetcher);
    const request = r.request('assets/worlds/empty.rhai');
    await Promise.resolve();
    expect(r.settled).not.toHaveBeenCalled();
    release('');
    await request;
    expect(fetcher.mock.calls[0][1].signal).toBeInstanceOf(AbortSignal);
    expect(r.bindings.wasm_push_world_toml).toHaveBeenCalledWith('assets/worlds/empty.rhai', '');
    expect(r.bindings.wasm_fail_world_fetch).not.toHaveBeenCalled();
    expect(r.settled).toHaveBeenCalledOnce();
  });

  it.each([404, 500])('retains required-source HTTP %s as failure instead of an empty script', async status => {
    const r = rig(async () => ({ ok: false, status }));
    await r.request('assets/worlds/missing.rhai');
    expect(r.bindings.wasm_fail_world_fetch).toHaveBeenCalledWith('assets/worlds/missing.rhai', `Error: HTTP ${status}`);
    expect(r.bindings.wasm_push_world_toml).not.toHaveBeenCalled();
    expect(r.settled).toHaveBeenCalledOnce();
  });

  it('settles a timed-out request as a terminal failure', async () => {
    const r = rig(async () => { throw new DOMException('timed out', 'TimeoutError'); });
    await r.request('assets/worlds/slow.toml');
    expect(r.bindings.wasm_fail_world_fetch).toHaveBeenCalledWith('assets/worlds/slow.toml', 'TimeoutError: timed out');
    expect(r.settled).toHaveBeenCalledOnce();
  });

  it('preserves optional sidecar absence', async () => {
    const r = rig(async () => ({ ok: false, status: 404 }));
    await r.request('assets/models/tier.model.toml', true);
    expect(r.bindings.wasm_push_sidecar_toml).toHaveBeenCalledWith('assets/models/tier.model.toml', '');
    expect(r.bindings.wasm_fail_world_fetch).not.toHaveBeenCalled();
  });
});
