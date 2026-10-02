// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import '../../gui/components/ph-helm-joystick.js';
import '../../gui/components/ph-lateral-thrust-joystick.js';

let frames, time, sequence;
beforeEach(() => {
  frames = new Map(); time = 100; sequence = 0;
  vi.spyOn(window, 'requestAnimationFrame').mockImplementation(cb => { frames.set(++sequence, cb); return sequence; });
  vi.spyOn(window, 'cancelAnimationFrame').mockImplementation(id => frames.delete(id));
  vi.spyOn(performance, 'now').mockImplementation(() => time);
  vi.spyOn(Date, 'now').mockImplementation(() => time);
  window.activateSemanticAction = vi.fn();
});
afterEach(() => { document.body.replaceChildren(); delete window.activateSemanticAction; vi.restoreAllMocks(); });
const tick = () => { const callbacks = [...frames.values()]; frames.clear(); callbacks.forEach(cb => cb(time)); };
const key = (type, code, target = document) => target.dispatchEvent(new KeyboardEvent(type, { code, bubbles: true, cancelable: true }));
const pointer = (target, type, pointerId = 7) => {
  const event = new MouseEvent(type, { clientX: 10, clientY: 10, bubbles: true, cancelable: true });
  Object.defineProperty(event, 'pointerId', { value: pointerId }); target.dispatchEvent(event);
};

describe.each([
  ['ph-helm-joystick', 'well', 2, false], ['ph-lateral-thrust-joystick', 'track', 1, true],
])('%s continuous input compatibility', (tag, targetId, sends, immediate) => {
  const mount = () => {
    const element = document.createElement(tag); document.body.append(element);
    const target = element.shadowRoot.getElementById(targetId);
    target.getBoundingClientRect = () => ({ left: 0, top: 0, width: 200, height: 200 });
    target.setPointerCapture = vi.fn(); target.releasePointerCapture = vi.fn();
    return { element, target };
  };
  it('preserves immediate pointer policy, heartbeat cadence and a single release', () => {
    const { target } = mount(); const send = window.activateSemanticAction;
    pointer(target, 'pointerdown'); expect(send).toHaveBeenCalledTimes(immediate ? sends : 0);
    tick(); expect(send).toHaveBeenCalledTimes((immediate ? 2 : 1) * sends);
    const count = send.mock.calls.length;
    time = 150; pointer(target, 'pointermove'); tick();
    expect(send).toHaveBeenCalledTimes(count + (immediate ? sends : 0));
    const before = send.mock.calls.length;
    pointer(target, 'lostpointercapture'); pointer(target, 'pointerup'); tick();
    expect(send).toHaveBeenCalledTimes(before + sends);
    expect(send.mock.calls.slice(-sends).every(([, payload]) => payload.value === 0)).toBe(true);
  });
  it('retains keyboard cadence, editable exclusions and blur neutralization', () => {
    mount(); const send = window.activateSemanticAction;
    const input = document.createElement('input'); document.body.append(input);
    key('keydown', 'ArrowLeft', input); tick(); expect(send).not.toHaveBeenCalled();
    key('keydown', 'ArrowLeft'); tick(); expect(send).toHaveBeenCalledTimes(sends);
    time = 150; tick(); expect(send).toHaveBeenCalledTimes(sends);
    time = 200; tick(); expect(send).toHaveBeenCalledTimes(sends * 2);
    window.dispatchEvent(new Event('blur')); tick(); tick();
    expect(send).toHaveBeenCalledTimes(sends * 3);
    expect(send.mock.calls.slice(-sends).every(([, payload]) => payload.value === 0)).toBe(true);
  });
  it('keeps heartbeat and keyboard clocks independent', () => {
    const { target } = mount(); const send = window.activateSemanticAction;
    vi.mocked(performance.now).mockReturnValue(50);
    vi.mocked(Date.now).mockReturnValue(500);
    key('keydown', 'ArrowLeft'); tick(); expect(send).toHaveBeenCalledTimes(sends);
    pointer(target, 'pointerdown'); tick();
    expect(send).toHaveBeenCalledTimes((immediate ? 2 : 1) * sends);
    vi.mocked(performance.now).mockReturnValue(100); tick();
    expect(send).toHaveBeenCalledTimes((immediate ? 3 : 2) * sends);
    pointer(target, 'pointerup'); tick();
    expect(send).toHaveBeenCalledTimes((immediate ? 4 : 3) * sends);
    vi.mocked(Date.now).mockReturnValue(600); tick();
    expect(send).toHaveBeenCalledTimes((immediate ? 5 : 4) * sends);
  });
  it('disconnects without neutral submission and reconnects retained key state once', () => {
    const { element } = mount(); const send = window.activateSemanticAction;
    key('keydown', 'ArrowLeft'); tick(); const count = send.mock.calls.length;
    element.remove(); expect(frames.size).toBe(0); expect(send).toHaveBeenCalledTimes(count);
    document.body.append(element); time = 200; key('keydown', 'ArrowLeft'); tick();
    expect(send).toHaveBeenCalledTimes(count + sends);
  });
  it('preserves pointer priority and Auto suppression', () => {
    const { element, target } = mount(); const send = window.activateSemanticAction;
    element.state = { auto: true }; key('keydown', 'ArrowLeft'); pointer(target, 'pointerdown'); tick();
    expect(send).not.toHaveBeenCalled();
    element.state = { auto: false }; key('keydown', 'ArrowLeft'); pointer(target, 'pointerdown'); tick();
    expect(send).toHaveBeenCalledTimes((immediate ? 2 : 1) * sends);
    expect(target.setPointerCapture).toHaveBeenCalledOnce();
  });
});
