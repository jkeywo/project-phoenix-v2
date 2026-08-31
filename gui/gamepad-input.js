/**
 * gui/gamepad-input.js — one explicitly-owned standard gamepad.
 *
 * Bindings name portable controls from the W3C "standard" mapping. Hardware
 * ids never enter a binding or selection: the browser slot is the local
 * preference, while an ephemeral generation prevents a new connection which
 * reuses that slot from inheriting the old connection's ownership.
 *
 * This module is DOM-free. Fabricated Gamepad snapshots drive the same poller
 * used by client.html, including selection, capture, edges and neutral gates.
 */

export const GAMEPAD_AXIS_CAPTURE_THRESHOLD = 0.5;

export const STANDARD_GAMEPAD_CONTROLS = Object.freeze({
  'face-bottom': Object.freeze({ input: 'button', index: 0,
    labelId: 'input.gamepad.face_bottom' }),
  'dpad-up': Object.freeze({ input: 'dpad', index: 12,
    labelId: 'input.gamepad.dpad_up' }),
  'dpad-down': Object.freeze({ input: 'dpad', index: 13,
    labelId: 'input.gamepad.dpad_down' }),
  'dpad-left': Object.freeze({ input: 'dpad', index: 14,
    labelId: 'input.gamepad.dpad_left' }),
  'dpad-right': Object.freeze({ input: 'dpad', index: 15,
    labelId: 'input.gamepad.dpad_right' }),
  'left-stick-x': Object.freeze({ input: 'axis', index: 0,
    negativeLabelId: 'input.gamepad.left_stick_left',
    positiveLabelId: 'input.gamepad.left_stick_right' }),
  'left-stick-y': Object.freeze({ input: 'axis', index: 1,
    negativeLabelId: 'input.gamepad.left_stick_up',
    positiveLabelId: 'input.gamepad.left_stick_down' }),
});

const CONTROL_ENTRIES = Object.entries(STANDARD_GAMEPAD_CONTROLS);

/** Normalize a portable standard-gamepad binding, or `null`. */
export function normalizeGamepadBinding(value) {
  if (value == null || value === '') return null;
  if (typeof value !== 'object' || value.type !== 'gamepad') {
    throw new TypeError('semantic gamepad binding requires type gamepad');
  }
  const control = typeof value.control === 'string' ? value.control.trim() : '';
  const mapped = STANDARD_GAMEPAD_CONTROLS[control];
  if (!mapped) throw new TypeError('semantic gamepad binding requires a standard control');
  const input = value.input == null ? mapped.input : String(value.input);
  if (input !== mapped.input) throw new TypeError('semantic gamepad binding input does not match control');
  if (mapped.input !== 'axis') {
    return Object.freeze({ type: 'gamepad', input: mapped.input, control });
  }
  const direction = value.direction === 'negative' ? 'negative'
    : value.direction === 'positive' ? 'positive' : '';
  if (!direction) throw new TypeError('semantic gamepad axis binding requires direction');
  const threshold = value.threshold == null
    ? GAMEPAD_AXIS_CAPTURE_THRESHOLD : Number(value.threshold);
  if (!Number.isFinite(threshold) || threshold <= 0 || threshold > 1) {
    throw new RangeError('semantic gamepad axis threshold must be above zero and at most one');
  }
  return Object.freeze({ type: 'gamepad', input: 'axis', control, direction, threshold });
}

/** Logical equality; no Gamepad.id or connection detail is part of identity. */
export function gamepadBindingsEqual(left, right) {
  if (!left || !right || left.type !== 'gamepad' || right.type !== 'gamepad') return false;
  try {
    const a = normalizeGamepadBinding(left);
    const b = normalizeGamepadBinding(right);
    return a.input === b.input && a.control === b.control
      && a.direction === b.direction && a.threshold === b.threshold;
  } catch (_) {
    return false;
  }
}

/** Localisable presentation tokens for the generic binding formatter. */
export function gamepadBindingDisplay(value) {
  const binding = normalizeGamepadBinding(value);
  if (!binding) return { labelId: null, values: {} };
  const mapped = STANDARD_GAMEPAD_CONTROLS[binding.control];
  if (binding.input !== 'axis') return { labelId: mapped.labelId, values: {} };
  return {
    labelId: binding.direction === 'negative'
      ? mapped.negativeLabelId : mapped.positiveLabelId,
    values: { threshold: String(binding.threshold) },
  };
}

function pressedButton(button) {
  return !!(button && (button.pressed || Number(button.value) >= 0.5));
}

/** Whether one logical binding is active on one standard-mapped snapshot. */
export function gamepadBindingPressed(value, gamepad) {
  if (!gamepad || gamepad.mapping !== 'standard') return false;
  const binding = normalizeGamepadBinding(value);
  if (!binding) return false;
  const mapped = STANDARD_GAMEPAD_CONTROLS[binding.control];
  if (binding.input !== 'axis') {
    return pressedButton((gamepad.buttons || [])[mapped.index]);
  }
  const axis = Number((gamepad.axes || [])[mapped.index]) || 0;
  return binding.direction === 'negative'
    ? axis <= -binding.threshold : axis >= binding.threshold;
}

/** Connected browser slots for Settings. Hardware names are intentionally absent. */
export function enumerateGamepads(snapshot) {
  return Array.from(snapshot || [])
    .filter(Boolean)
    .map((pad) => ({ index: Number(pad.index), supported: pad.mapping === 'standard' }))
    .filter((pad) => Number.isInteger(pad.index) && pad.index >= 0)
    .sort((a, b) => a.index - b.index);
}

function firstCaptureBinding(gamepad) {
  if (!gamepad || gamepad.mapping !== 'standard') return null;
  for (const [control, mapped] of CONTROL_ENTRIES) {
    if (mapped.input !== 'axis' && pressedButton((gamepad.buttons || [])[mapped.index])) {
      return normalizeGamepadBinding({ type: 'gamepad', input: mapped.input, control });
    }
  }
  for (const [control, mapped] of CONTROL_ENTRIES) {
    if (mapped.input !== 'axis') continue;
    const value = Number((gamepad.axes || [])[mapped.index]) || 0;
    if (value <= -GAMEPAD_AXIS_CAPTURE_THRESHOLD) {
      return normalizeGamepadBinding({
        type: 'gamepad', input: 'axis', control, direction: 'negative',
        threshold: GAMEPAD_AXIS_CAPTURE_THRESHOLD,
      });
    }
    if (value >= GAMEPAD_AXIS_CAPTURE_THRESHOLD) {
      return normalizeGamepadBinding({
        type: 'gamepad', input: 'axis', control, direction: 'positive',
        threshold: GAMEPAD_AXIS_CAPTURE_THRESHOLD,
      });
    }
  }
  return null;
}

function isNeutral(gamepad) {
  return firstCaptureBinding(gamepad) === null;
}

function bindingKey(binding) {
  const normalized = normalizeGamepadBinding(binding);
  return normalized ? JSON.stringify(normalized) : '';
}

/**
 * Create the one parent-realm gamepad runtime.
 *
 * `poll()` accepts a fabricated snapshot for tests. In production `start()`
 * takes exactly one `navigator.getGamepads()` snapshot per animation frame.
 */
export function createGamepadInputRuntime(options = {}) {
  const getGamepads = typeof options.getGamepads === 'function'
    ? options.getGamepads
    : () => (typeof navigator !== 'undefined' && navigator.getGamepads
      ? navigator.getGamepads() : []);
  const getContext = typeof options.getContext === 'function' ? options.getContext : () => null;
  const getActions = typeof options.getActions === 'function' ? options.getActions : () => [];
  const activate = typeof options.activate === 'function' ? options.activate : () => false;
  const onCapture = typeof options.onCapture === 'function' ? options.onCapture : () => {};
  const onStateChange = typeof options.onStateChange === 'function'
    ? options.onStateChange : () => {};
  const frame = typeof options.requestAnimationFrame === 'function'
    ? options.requestAnimationFrame
    : (callback) => requestAnimationFrame(callback);
  const cancelFrame = typeof options.cancelAnimationFrame === 'function'
    ? options.cancelAnimationFrame
    : (handle) => cancelAnimationFrame(handle);

  const connections = new Map();
  let devices = [];
  let selection = null;
  let captureTarget = null;
  let previousPressed = new Set();
  let neutralGate = false;
  let lastContext = null;
  let lastStateKey = '';
  let frameHandle = null;
  let latestSnapshot = [];

  function connection(index) {
    if (!connections.has(index)) connections.set(index, { connected: false, generation: 0 });
    return connections.get(index);
  }

  function observe(snapshot) {
    latestSnapshot = Array.from(snapshot || []);
    const present = new Set();
    for (const device of enumerateGamepads(latestSnapshot)) {
      present.add(device.index);
      const record = connection(device.index);
      if (!record.connected) {
        record.connected = true;
        record.generation += 1;
      }
    }
    for (const [index, record] of connections) {
      if (record.connected && !present.has(index)) record.connected = false;
    }
    devices = enumerateGamepads(latestSnapshot);
  }

  function selectedPad() {
    if (!selection) return null;
    const record = connection(selection.index);
    if (!record.connected || record.generation !== selection.generation) return null;
    const pad = latestSnapshot[selection.index];
    return pad && Number(pad.index) === selection.index ? pad : null;
  }

  function status() {
    if (!selection) return 'none';
    const pad = selectedPad();
    if (!pad) return 'disconnected';
    if (pad.mapping !== 'standard') return 'unsupported';
    if (neutralGate) return 'neutral';
    return 'ready';
  }

  function state() {
    return {
      devices: devices.map((device) => ({ ...device })),
      selectedIndex: selection ? selection.index : null,
      status: status(),
      capturing: captureTarget ? { ...captureTarget } : null,
    };
  }

  function notify() {
    const next = state();
    const key = JSON.stringify(next);
    if (key === lastStateKey) return;
    lastStateKey = key;
    onStateChange(next);
  }

  function neutralize() {
    previousPressed.clear();
    neutralGate = !!selection;
    notify();
  }

  function select(index) {
    if (index == null || index === '') {
      selection = null;
      captureTarget = null;
      previousPressed.clear();
      neutralGate = false;
      notify();
      return { status: 'none' };
    }
    observe(getGamepads());
    const slot = Number(index);
    const pad = latestSnapshot[slot];
    const record = connection(slot);
    if (!Number.isInteger(slot) || !pad || !record.connected || pad.mapping !== 'standard') {
      notify();
      return { status: pad ? 'unsupported' : 'disconnected' };
    }
    selection = { index: slot, generation: record.generation };
    captureTarget = null;
    neutralize();
    return { status: 'selected', index: slot };
  }

  function beginCapture(actionId, slot) {
    captureTarget = { actionId: String(actionId), slot: Number(slot) };
    neutralize();
  }

  function endCapture(actionId, slot) {
    if (!captureTarget) return;
    if (actionId != null && (captureTarget.actionId !== String(actionId)
      || captureTarget.slot !== Number(slot))) return;
    captureTarget = null;
    neutralize();
  }

  function noteDisconnected(index) {
    const record = connection(Number(index));
    record.connected = false;
    previousPressed.clear();
    neutralGate = !!selection;
    notify();
  }

  function noteConnected(gamepad) {
    if (!gamepad || !Number.isInteger(Number(gamepad.index))) return;
    const record = connection(Number(gamepad.index));
    // A browser connection event is a new ephemeral connection even when a
    // missed poll never observed the old slot empty.
    record.connected = true;
    record.generation += 1;
    previousPressed.clear();
    neutralGate = !!selection;
    notify();
  }

  function poll(snapshot = getGamepads()) {
    observe(snapshot);
    const context = getContext() || null;
    if (context !== lastContext) {
      lastContext = context;
      neutralize();
    }
    const pad = selectedPad();
    if (!pad || pad.mapping !== 'standard') {
      previousPressed.clear();
      notify();
      return state();
    }
    if (neutralGate) {
      if (isNeutral(pad)) neutralGate = false;
      previousPressed.clear();
      notify();
      return state();
    }
    if (captureTarget) {
      const binding = firstCaptureBinding(pad);
      if (binding) {
        const target = captureTarget;
        captureTarget = null;
        neutralize();
        onCapture(target, binding);
      }
      notify();
      return state();
    }

    const nextPressed = new Set();
    for (const action of getActions(context) || []) {
      if (!action || !Array.isArray(action.contexts) || !action.contexts.includes(context)) continue;
      for (const binding of action.bindings || []) {
        if (!binding || binding.type !== 'gamepad' || !gamepadBindingPressed(binding, pad)) continue;
        const key = bindingKey(binding);
        nextPressed.add(key);
        if (!previousPressed.has(key)) {
          activate(action.id, { context, source: 'gamepad', binding: { ...binding } });
        }
      }
    }
    previousPressed = nextPressed;
    notify();
    return state();
  }

  function start(win = typeof window !== 'undefined' ? window : null) {
    if (frameHandle !== null) return;
    const connected = (event) => noteConnected(event && event.gamepad);
    const disconnected = (event) => noteDisconnected(event && event.gamepad && event.gamepad.index);
    if (win && win.addEventListener) {
      win.addEventListener('gamepadconnected', connected);
      win.addEventListener('gamepaddisconnected', disconnected);
    }
    const tick = () => {
      poll();
      frameHandle = frame(tick);
    };
    poll();
    frameHandle = frame(tick);
    return () => {
      if (frameHandle !== null) cancelFrame(frameHandle);
      frameHandle = null;
      if (win && win.removeEventListener) {
        win.removeEventListener('gamepadconnected', connected);
        win.removeEventListener('gamepaddisconnected', disconnected);
      }
    };
  }

  return {
    poll, start, state, select, beginCapture, endCapture, neutralize,
    noteConnected, noteDisconnected,
  };
}

if (typeof window !== 'undefined') {
  window.createGamepadInputRuntime = createGamepadInputRuntime;
}
