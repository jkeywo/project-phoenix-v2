// @vitest-environment jsdom
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { installKeyRelay } from '../../gui/key-relay.js';

describe('installKeyRelay', () => {
  let frame;
  let target;
  let uninstall;

  // A real iframe: its contentDocument is a separate realm with its own
  // defaultView, which is what the relay constructs the copied event in.
  beforeEach(() => {
    document.body.innerHTML = '';
    frame = document.createElement('iframe');
    document.body.appendChild(frame);
    target = frame.contentDocument;
    uninstall = null;
  });

  afterEach(() => {
    if (uninstall) uninstall();
    document.body.innerHTML = '';
  });

  function install(getTargetDoc) {
    uninstall = installKeyRelay(document, getTargetDoc || (() => target));
  }

  function hostKey(type, init) {
    const e = new KeyboardEvent(type, Object.assign(
      { code: 'KeyW', bubbles: true, cancelable: true }, init));
    document.dispatchEvent(e);
    return e;
  }

  it('relays keydown and keyup to the active console document', () => {
    const seen = [];
    target.addEventListener('keydown', (e) => seen.push(['keydown', e.code]));
    target.addEventListener('keyup', (e) => seen.push(['keyup', e.code]));
    install();

    hostKey('keydown', { code: 'ArrowUp' });
    hostKey('keyup', { code: 'ArrowUp' });

    expect(seen).toEqual([['keydown', 'ArrowUp'], ['keyup', 'ArrowUp']]);
  });

  it('preserves code, repeat and modifier state', () => {
    const seen = [];
    target.addEventListener('keydown', (e) => seen.push({
      code: e.code, key: e.key, repeat: e.repeat,
      shiftKey: e.shiftKey, ctrlKey: e.ctrlKey,
    }));
    install();

    hostKey('keydown', { code: 'ShiftLeft', key: 'Shift', shiftKey: true });
    hostKey('keydown', { code: 'ControlLeft', key: 'Control', ctrlKey: true, repeat: true });

    expect(seen).toEqual([
      { code: 'ShiftLeft', key: 'Shift', repeat: false, shiftKey: true, ctrlKey: false },
      { code: 'ControlLeft', key: 'Control', repeat: true, shiftKey: false, ctrlKey: true },
    ]);
  });

  it('delivers a genuine KeyboardEvent in the target frame realm', () => {
    let ok = null;
    target.addEventListener('keydown', (e) => {
      ok = e instanceof frame.contentWindow.KeyboardEvent;
    });
    install();
    hostKey('keydown');
    expect(ok).toBe(true);
  });

  it('does not relay while the operator types in a host-page field', () => {
    const onKey = vi.fn();
    target.addEventListener('keydown', onKey);
    install();

    const input = document.createElement('input');
    document.body.appendChild(input);
    input.dispatchEvent(new KeyboardEvent('keydown', { code: 'KeyW', bubbles: true }));

    expect(onKey).not.toHaveBeenCalled();
  });

  it('does nothing when no console is active', () => {
    const onKey = vi.fn();
    target.addEventListener('keydown', onKey);
    install(() => null);
    expect(() => hostKey('keydown')).not.toThrow();
    expect(onKey).not.toHaveBeenCalled();
  });

  it('refuses to relay a document to itself', () => {
    const onKey = vi.fn();
    document.addEventListener('keydown', onKey);
    install(() => document);
    hostKey('keydown');
    // Once for the original event only — no recursive re-dispatch.
    expect(onKey).toHaveBeenCalledTimes(1);
    document.removeEventListener('keydown', onKey);
  });

  it('survives a getTargetDoc that throws (cross-origin frame)', () => {
    install(() => { throw new Error('cross-origin'); });
    expect(() => hostKey('keydown')).not.toThrow();
  });

  it('mirrors the console preventDefault onto the host event', () => {
    target.addEventListener('keydown', (e) => e.preventDefault());
    install();
    const hostEvent = hostKey('keydown', { code: 'ArrowUp' });
    // Otherwise arrows claimed by the joystick would still scroll the host.
    expect(hostEvent.defaultPrevented).toBe(true);
  });

  it('leaves the host event alone when the console ignores the key', () => {
    install();
    const hostEvent = hostKey('keydown', { code: 'KeyQ' });
    expect(hostEvent.defaultPrevented).toBe(false);
  });

  it('stops relaying after uninstall', () => {
    const onKey = vi.fn();
    target.addEventListener('keydown', onKey);
    install();
    hostKey('keydown');
    expect(onKey).toHaveBeenCalledTimes(1);

    uninstall();
    uninstall = null;
    hostKey('keydown');
    expect(onKey).toHaveBeenCalledTimes(1);
  });

  it('returns a safe no-op when given no document or resolver', () => {
    expect(() => installKeyRelay(null, () => target)()).not.toThrow();
    expect(() => installKeyRelay(document, null)()).not.toThrow();
  });
});
