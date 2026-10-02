import { describe, it, expect, vi } from 'vitest';
import { push, setOverlay } from '../../gui/iframe-bridge.js';

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

// ── setOverlay (issue #1373) ─────────────────────────────────────────────────

describe('setOverlay', () => {
  it('calls __setConsoleOverlay(id) on the iframe contentWindow', () => {
    const fn = vi.fn();
    const iframe = { contentWindow: { __setConsoleOverlay: fn } };
    setOverlay(iframe, 'intel-overlay');
    expect(fn).toHaveBeenCalledWith('intel-overlay');
  });

  it('normalises every "nothing selected" spelling to null', () => {
    const fn = vi.fn();
    const iframe = { contentWindow: { __setConsoleOverlay: fn } };
    setOverlay(iframe, null);
    setOverlay(iframe, undefined);
    setOverlay(iframe, '');
    expect(fn.mock.calls).toEqual([[null], [null], [null]]);
  });

  it('does nothing when iframeEl is null', () => {
    expect(() => setOverlay(null, 'intel-overlay')).not.toThrow();
  });

  it('does nothing when iframeEl has no contentWindow', () => {
    expect(() => setOverlay({}, 'intel-overlay')).not.toThrow();
  });

  it('does nothing when the console declared no overlays, so installed no hook', () => {
    const iframe = { contentWindow: {} };
    expect(() => setOverlay(iframe, 'intel-overlay')).not.toThrow();
  });

  it('does nothing when __setConsoleOverlay is not a function', () => {
    const iframe = { contentWindow: { __setConsoleOverlay: 'not-a-function' } };
    expect(() => setOverlay(iframe, 'intel-overlay')).not.toThrow();
  });

  it('swallows errors thrown by __setConsoleOverlay', () => {
    const iframe = {
      contentWindow: { __setConsoleOverlay: () => { throw new Error('oops'); } },
    };
    expect(() => setOverlay(iframe, 'intel-overlay')).not.toThrow();
  });
});
