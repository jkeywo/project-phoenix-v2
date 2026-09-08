import { STANDARD_GAMEPAD_CONTROLS } from './gamepad-input.js';

/** Only hide a touch surface when every axis it supplies has a usable binding. */
export function helmTouchVisibility(actions, gamepadState, hideTouchControls = true) {
  const connected = gamepadState?.connected === true;
  const controls = new Set(gamepadState?.controls || []);
  const usable = (id) => (actions || []).some((action) => action.id === id
    && action.continuous && action.bindings?.some((binding) => binding?.type === 'gamepad'
      && binding.input === 'axis' && binding.direction == null
      && STANDARD_GAMEPAD_CONTROLS[binding.control]?.input === 'axis'
      && controls.has(binding.control)));
  return {
    joystick: !(hideTouchControls && connected && usable('helm.thrust') && usable('helm.steering')),
    lateral: !(hideTouchControls && connected && usable('helm.lateral-thrust')),
  };
}

const hiddenDisplays = new WeakMap();

export function applyHelmTouchVisibility(doc, visibility) {
  if (!doc) return;
  for (const [tag, visible] of [['ph-helm-joystick', visibility.joystick],
    ['ph-lateral-thrust-joystick', visibility.lateral]]) {
    for (const control of doc.querySelectorAll(tag)) {
      if (visible) {
        const original = hiddenDisplays.get(control);
        if (!original) continue;
        if (original.value) control.style.setProperty('display', original.value, original.priority);
        else control.style.removeProperty('display');
        hiddenDisplays.delete(control);
      } else {
        if (!hiddenDisplays.has(control)) hiddenDisplays.set(control, {
          value: control.style.getPropertyValue('display'),
          priority: control.style.getPropertyPriority('display'),
        });
        control.style.setProperty('display', 'none', 'important');
      }
    }
  }
}

if (typeof window !== 'undefined') {
  window.GamepadPresentation = { helmTouchVisibility, applyHelmTouchVisibility };
}
