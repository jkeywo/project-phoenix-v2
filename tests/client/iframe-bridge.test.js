import { describe, it, expect, vi } from 'vitest';
import { push, wireLoad } from '../../gui/iframe-bridge.js';

// ── push ─────────────────────────────────────────────────────────────────────

describe('push', () => {
  it('calls __updateConsole(name, json) on the iframe contentWindow', () => {
    const fn = vi.fn();
    const iframe = { contentWindow: { __updateConsole: fn } };
    push(iframe, 'Tactical', '{"banks":[]}');
    expect(fn).toHaveBeenCalledWith('Tactical', '{"banks":[]}');
  });

  it('does nothing when iframeEl is null', () => {
    // Should not throw
    expect(() => push(null, 'Helm', '{}')).not.toThrow();
  });

  it('does nothing when iframeEl has no contentWindow', () => {
    expect(() => push({}, 'Helm', '{}')).not.toThrow();
  });

  it('does nothing when contentWindow has no __updateConsole', () => {
    const iframe = { contentWindow: {} };
    expect(() => push(iframe, 'Helm', '{}')).not.toThrow();
  });

  it('does nothing when __updateConsole is not a function', () => {
    const iframe = { contentWindow: { __updateConsole: 'not-a-function' } };
    expect(() => push(iframe, 'Helm', '{}')).not.toThrow();
  });

  it('swallows errors thrown by __updateConsole', () => {
    const iframe = { contentWindow: { __updateConsole: () => { throw new Error('oops'); } } };
    expect(() => push(iframe, 'Repair', '{}')).not.toThrow();
  });
});

// ── wireLoad ─────────────────────────────────────────────────────────────────

describe('wireLoad', () => {
  it('attaches a load listener to the iframe', () => {
    const addFn = vi.fn();
    const iframe = { addEventListener: addFn };
    const refresh = vi.fn();
    wireLoad(iframe, refresh);
    expect(addFn).toHaveBeenCalledWith('load', refresh);
  });

  it('does nothing when iframeEl is null', () => {
    expect(() => wireLoad(null, vi.fn())).not.toThrow();
  });

  it('does nothing when iframeEl is undefined', () => {
    expect(() => wireLoad(undefined, vi.fn())).not.toThrow();
  });

  it('calls the refresh function when the load event fires', () => {
    let loadCb = null;
    const iframe = { addEventListener: (ev, cb) => { if (ev === 'load') loadCb = cb; } };
    const refresh = vi.fn();
    wireLoad(iframe, refresh);
    loadCb();
    expect(refresh).toHaveBeenCalledTimes(1);
  });
});
