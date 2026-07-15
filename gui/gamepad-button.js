/**
 * gui/gamepad-button.js — edge-triggered gamepad button watching.
 *
 * The Gamepad API is poll-only: it emits no button press/release events, so a
 * rAF loop has to sample `navigator.getGamepads()` and diff against the last
 * frame. This module wraps that so console components can treat a pad button
 * like any other press/release source. `ph-helm-joystick.js` polls the sticks
 * the same way inside its own input loop.
 *
 * Guards on `window`/`navigator.getGamepads` so importing in Node (tests) and
 * running in browsers without gamepad support are both safe.
 */

/** Standard-mapping face button indices (https://w3c.github.io/gamepad/#remapping). */
export const GAMEPAD_BUTTON = { A: 0, B: 1, X: 2, Y: 3 };

/**
 * Call `onChange(pressed)` whenever gamepad button `index` changes state on
 * any connected pad. Fires only on transitions, never per-frame while held.
 *
 * The polling loop runs only while a pad is connected (or a press is still
 * latched), so consoles on a keyboard-only machine cost nothing per frame.
 *
 * @param {number} index — button index, e.g. GAMEPAD_BUTTON.A.
 * @param {(pressed: boolean) => void} onChange
 * @param {Window} [win=window]
 * @returns {() => void} stop function; releases a held press before detaching.
 */
export function observeGamepadButton(index, onChange, win) {
  win = win || (typeof window !== 'undefined' ? window : null);
  if (!win || !win.navigator || typeof win.navigator.getGamepads !== 'function') {
    return () => {};
  }

  let raf = null;
  let pressed = false;

  const pads = () => win.navigator.getGamepads() || [];

  const hasPad = () => {
    const list = pads();
    for (let i = 0; i < list.length; i++) if (list[i]) return true;
    return false;
  };

  const isPressed = () => {
    const list = pads();
    for (let i = 0; i < list.length; i++) {
      const gp = list[i];
      if (!gp || !gp.buttons || gp.buttons.length <= index) continue;
      const b = gp.buttons[index];
      if (b == null) continue;
      // Buttons are GamepadButton objects, but some older/odd drivers hand
      // back bare analog numbers.
      if (typeof b === 'object' ? b.pressed : b > 0.5) return true;
    }
    return false;
  };

  const loop = () => {
    raf = null;
    const now = isPressed();
    if (now !== pressed) {
      pressed = now;
      onChange(pressed);
    }
    // Keep polling while a pad is attached. The `pressed` term matters when a
    // pad is yanked mid-hold: the next frame sees no pad, reports the release,
    // and only then lets the loop stop.
    if (hasPad() || pressed) raf = win.requestAnimationFrame(loop);
  };

  const start = () => { if (raf === null) raf = win.requestAnimationFrame(loop); };

  win.addEventListener('gamepadconnected', start);
  if (hasPad()) start();

  return () => {
    win.removeEventListener('gamepadconnected', start);
    if (raf !== null) { win.cancelAnimationFrame(raf); raf = null; }
    if (pressed) { pressed = false; onChange(false); }
  };
}
