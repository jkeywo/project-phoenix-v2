import { describe, it, expect, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';

const source = readFileSync(new URL('../../gui/native-pane-raf.js', import.meta.url), 'utf8');
const keepalive = readFileSync(new URL('../../gui/bg-raf-keepalive.js', import.meta.url), 'utf8');

function fixture({ native = true } = {}) {
  let next = 0;
  const pending = new Map();
  const host = {
    PhoenixOperatorCapabilities: { surface: native ? 'native-pane' : 'browser' },
    requestAnimationFrame: vi.fn(function (cb) {
      expect(this).toBe(host);
      pending.set(++next, cb);
      return next;
    }),
    cancelAnimationFrame: vi.fn(function (id) {
      expect(this).toBe(host);
      pending.delete(id);
    }),
  };
  const engineRequest = vi.fn(() => 7);
  const engineCancel = vi.fn();
  const win = {
    parent: host,
    performance: { now: () => 42 },
    requestAnimationFrame: engineRequest,
    cancelAnimationFrame: engineCancel,
  };
  const context = { window: win };
  return { host, win, pending, engineRequest, engineCancel, context };
}

describe('native Station iframe animation scheduler', () => {
  it('uses the parent scheduler with the child clock and matching cancellation', () => {
    const f = fixture();
    runInNewContext(source, f.context);
    const seen = [];
    const id = f.win.requestAnimationFrame(stamp => seen.push(stamp));
    f.pending.get(id)(90000);
    expect(seen).toEqual([42]);
    expect(f.engineRequest).not.toHaveBeenCalled();

    const cancelled = f.win.requestAnimationFrame(() => seen.push('cancelled'));
    // Cancellation belongs to the same captured scheduler even if a later
    // script replaces the parent's public methods.
    const cancel = f.host.cancelAnimationFrame;
    f.host.cancelAnimationFrame = vi.fn();
    f.win.cancelAnimationFrame(cancelled);
    expect(cancel).toHaveBeenCalledWith(cancelled);
    expect(f.pending.has(cancelled)).toBe(false);
    expect(f.engineCancel).not.toHaveBeenCalled();
  });

  it('reaches the parent after Helm has captured its native engine in the classic keepalive', () => {
    const f = fixture();
    Object.assign(f.context, {
      document: { hidden: false },
      performance: f.win.performance,
      MessageChannel: class {
        port1 = { postMessage: vi.fn() };
        port2 = {};
      },
    });
    runInNewContext(keepalive, f.context);
    runInNewContext(source, f.context);
    f.win.requestAnimationFrame(() => {});
    expect(f.host.requestAnimationFrame).toHaveBeenCalledOnce();
    expect(f.engineRequest).not.toHaveBeenCalled();
  });

  it.each(['top-level', 'ordinary iframe', 'cross-origin iframe'])('leaves %s scheduling unchanged', kind => {
    const f = fixture({ native: false });
    if (kind === 'top-level') f.win.parent = f.win;
    if (kind === 'cross-origin iframe') {
      Object.defineProperty(f.host, 'PhoenixOperatorCapabilities', {
        get() { throw new Error('SecurityError'); },
      });
    }
    runInNewContext(source, f.context);
    expect(f.win.requestAnimationFrame).toBe(f.engineRequest);
    expect(f.win.cancelAnimationFrame).toBe(f.engineCancel);
  });

  it('is safe in plain Node component imports', () => {
    expect(() => runInNewContext(source, {})).not.toThrow();
  });
});
