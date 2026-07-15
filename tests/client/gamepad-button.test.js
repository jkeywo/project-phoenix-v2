// @vitest-environment jsdom
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { observeGamepadButton, GAMEPAD_BUTTON } from '../../gui/gamepad-button.js';

// A fake pad whose buttons can be poked between rAF frames.
function pad(pressedIndices) {
  const buttons = [];
  for (let i = 0; i < 4; i++) buttons.push({ pressed: pressedIndices.includes(i) });
  return { buttons, axes: [0, 0] };
}

describe('observeGamepadButton', () => {
  let pads;
  let frames;

  // Drive rAF by hand so each `tick()` is exactly one polled frame.
  function tick() {
    const due = frames;
    frames = [];
    for (const fn of due) fn();
  }

  beforeEach(() => {
    pads = [];
    frames = [];
    navigator.getGamepads = () => pads;
    vi.spyOn(window, 'requestAnimationFrame').mockImplementation((fn) => {
      frames.push(fn);
      return frames.length;
    });
    vi.spyOn(window, 'cancelAnimationFrame').mockImplementation(() => { frames = []; });
  });

  afterEach(() => {
    vi.restoreAllMocks();
    delete navigator.getGamepads;
  });

  it('reports press and release once each, not per frame', () => {
    pads = [pad([])];
    const onChange = vi.fn();
    observeGamepadButton(GAMEPAD_BUTTON.A, onChange);

    tick();
    expect(onChange).not.toHaveBeenCalled();

    pads = [pad([GAMEPAD_BUTTON.A])];
    tick();
    tick();
    tick();
    expect(onChange).toHaveBeenCalledTimes(1);
    expect(onChange).toHaveBeenCalledWith(true);

    pads = [pad([])];
    tick();
    expect(onChange).toHaveBeenCalledTimes(2);
    expect(onChange).toHaveBeenLastCalledWith(false);
  });

  it('distinguishes A from B', () => {
    pads = [pad([GAMEPAD_BUTTON.B])];
    const onA = vi.fn();
    const onB = vi.fn();
    observeGamepadButton(GAMEPAD_BUTTON.A, onA);
    observeGamepadButton(GAMEPAD_BUTTON.B, onB);
    tick();
    expect(onA).not.toHaveBeenCalled();
    expect(onB).toHaveBeenCalledWith(true);
  });

  it('does not poll until a pad connects', () => {
    const onChange = vi.fn();
    observeGamepadButton(GAMEPAD_BUTTON.A, onChange);
    expect(frames).toHaveLength(0);

    pads = [pad([GAMEPAD_BUTTON.A])];
    window.dispatchEvent(new Event('gamepadconnected'));
    tick();
    expect(onChange).toHaveBeenCalledWith(true);
  });

  it('reports a release when the pad is unplugged mid-hold', () => {
    pads = [pad([GAMEPAD_BUTTON.A])];
    const onChange = vi.fn();
    observeGamepadButton(GAMEPAD_BUTTON.A, onChange);
    tick();
    expect(onChange).toHaveBeenLastCalledWith(true);

    pads = [];
    tick();
    expect(onChange).toHaveBeenLastCalledWith(false);
  });

  it('stop() releases a held button and halts polling', () => {
    pads = [pad([GAMEPAD_BUTTON.A])];
    const onChange = vi.fn();
    const stop = observeGamepadButton(GAMEPAD_BUTTON.A, onChange);
    tick();
    expect(onChange).toHaveBeenLastCalledWith(true);

    stop();
    expect(onChange).toHaveBeenLastCalledWith(false);
    expect(onChange).toHaveBeenCalledTimes(2);

    window.dispatchEvent(new Event('gamepadconnected'));
    tick();
    expect(onChange).toHaveBeenCalledTimes(2);
  });

  it('is a no-op stop function when the browser has no gamepad support', () => {
    delete navigator.getGamepads;
    const onChange = vi.fn();
    const stop = observeGamepadButton(GAMEPAD_BUTTON.A, onChange);
    expect(() => stop()).not.toThrow();
    expect(onChange).not.toHaveBeenCalled();
  });
});
