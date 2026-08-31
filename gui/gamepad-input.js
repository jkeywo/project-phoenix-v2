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
  'face-right': Object.freeze({ input: 'button', index: 1,
    labelId: 'input.gamepad.face_right' }),
  'face-left': Object.freeze({ input: 'button', index: 2,
    labelId: 'input.gamepad.face_left' }),
  'face-top': Object.freeze({ input: 'button', index: 3,
    labelId: 'input.gamepad.face_top' }),
  'left-shoulder': Object.freeze({ input: 'button', index: 4,
    labelId: 'input.gamepad.left_shoulder' }),
  'right-shoulder': Object.freeze({ input: 'button', index: 5,
    labelId: 'input.gamepad.right_shoulder' }),
  'left-trigger': Object.freeze({ input: 'button', index: 6,
    labelId: 'input.gamepad.left_trigger' }),
  'right-trigger': Object.freeze({ input: 'button', index: 7,
    labelId: 'input.gamepad.right_trigger' }),
  select: Object.freeze({ input: 'button', index: 8,
    labelId: 'input.gamepad.select' }),
  start: Object.freeze({ input: 'button', index: 9,
    labelId: 'input.gamepad.start' }),
  'left-stick-button': Object.freeze({ input: 'button', index: 10,
    labelId: 'input.gamepad.left_stick_button' }),
  'right-stick-button': Object.freeze({ input: 'button', index: 11,
    labelId: 'input.gamepad.right_stick_button' }),
  'dpad-up': Object.freeze({ input: 'dpad', index: 12,
    labelId: 'input.gamepad.dpad_up' }),
  'dpad-down': Object.freeze({ input: 'dpad', index: 13,
    labelId: 'input.gamepad.dpad_down' }),
  'dpad-left': Object.freeze({ input: 'dpad', index: 14,
    labelId: 'input.gamepad.dpad_left' }),
  'dpad-right': Object.freeze({ input: 'dpad', index: 15,
    labelId: 'input.gamepad.dpad_right' }),
  'left-stick-x': Object.freeze({ input: 'axis', index: 0,
    labelId: 'input.gamepad.left_stick_x',
    negativeLabelId: 'input.gamepad.left_stick_left',
    positiveLabelId: 'input.gamepad.left_stick_right' }),
  'left-stick-y': Object.freeze({ input: 'axis', index: 1,
    labelId: 'input.gamepad.left_stick_y',
    negativeLabelId: 'input.gamepad.left_stick_up',
    positiveLabelId: 'input.gamepad.left_stick_down' }),
  'right-stick-x': Object.freeze({ input: 'axis', index: 2,
    labelId: 'input.gamepad.right_stick_x',
    negativeLabelId: 'input.gamepad.right_stick_left',
    positiveLabelId: 'input.gamepad.right_stick_right' }),
  'right-stick-y': Object.freeze({ input: 'axis', index: 3,
    labelId: 'input.gamepad.right_stick_y',
    negativeLabelId: 'input.gamepad.right_stick_up',
    positiveLabelId: 'input.gamepad.right_stick_down' }),
});

const CONTROL_ENTRIES = Object.entries(STANDARD_GAMEPAD_CONTROLS);

/** Normalize a portable standard-gamepad binding, or `null`. */
export function normalizeGamepadBinding(value, options = {}) {
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
  if (options.continuous === true) {
    return Object.freeze({ type: 'gamepad', input: 'axis', control });
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
    const a = normalizeGamepadBinding(left, {
      continuous: left.input === 'axis' && left.direction == null,
    });
    const b = normalizeGamepadBinding(right, {
      continuous: right.input === 'axis' && right.direction == null,
    });
    if (a.input === 'axis' && b.input === 'axis' && a.control === b.control
        && (a.direction == null || b.direction == null)) return true;
    return a.input === b.input && a.control === b.control
      && (a.input !== 'axis' || a.direction === b.direction);
  } catch (_) {
    return false;
  }
}

/** Localisable presentation tokens for the generic binding formatter. */
export function gamepadBindingDisplay(value) {
  const binding = normalizeGamepadBinding(value, {
    continuous: value && value.input === 'axis' && value.direction == null,
  });
  if (!binding) return { labelId: null, values: {} };
  const mapped = STANDARD_GAMEPAD_CONTROLS[binding.control];
  if (binding.input !== 'axis') return { labelId: mapped.labelId, values: {} };
  return binding.direction == null ? { labelId: mapped.labelId, values: {} } : {
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

function firstCaptureBinding(gamepad, options = {}) {
  if (!gamepad || gamepad.mapping !== 'standard') return null;
  if (options.continuous !== true) {
    for (const [control, mapped] of CONTROL_ENTRIES) {
      if (mapped.input !== 'axis' && pressedButton((gamepad.buttons || [])[mapped.index])) {
        return normalizeGamepadBinding({ type: 'gamepad', input: mapped.input, control });
      }
    }
  }
  for (const [control, mapped] of CONTROL_ENTRIES) {
    if (mapped.input !== 'axis') continue;
    const value = Number((gamepad.axes || [])[mapped.index]) || 0;
    if (options.continuous === true && Math.abs(value) >= GAMEPAD_AXIS_CAPTURE_THRESHOLD) {
      return normalizeGamepadBinding({
        type: 'gamepad', input: 'axis', control,
      }, { continuous: true });
    }
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

function bindingKey(binding) {
  const normalized = normalizeGamepadBinding(binding, {
    continuous: binding && binding.input === 'axis' && binding.direction == null,
  });
  return normalized ? JSON.stringify(normalized) : '';
}

/** Smoothly rescale one raw axis around a configurable centre deadzone. */
export function normalizeContinuousAxis(rawValue, tuning = {}, continuous = {}) {
  const raw = Math.max(-1, Math.min(1, Number(rawValue) || 0));
  const deadzone = Number(tuning.deadzone);
  const safeDeadzone = Number.isFinite(deadzone) && deadzone >= 0 && deadzone < 1
    ? deadzone : 0;
  const magnitude = Math.abs(raw);
  let normalized = magnitude <= safeDeadzone
    ? 0 : Math.sign(raw) * ((magnitude - safeDeadzone) / (1 - safeDeadzone));
  if (tuning.inverted === true) normalized = -normalized;

  const min = Number.isFinite(continuous.min) ? continuous.min : -1;
  const max = Number.isFinite(continuous.max) ? continuous.max : 1;
  const neutral = Number.isFinite(continuous.neutral) ? continuous.neutral : 0;
  return normalized >= 0
    ? neutral + normalized * (max - neutral)
    : neutral + (-normalized) * (min - neutral);
}

function continuousDeflection(value, continuous) {
  const neutral = continuous.neutral;
  if (value === neutral) return 0;
  return value > neutral
    ? (value - neutral) / (continuous.max - neutral)
    : (neutral - value) / (neutral - continuous.min);
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
  const isTransportLive = typeof options.isTransportLive === 'function'
    ? options.isTransportLive : () => true;
  const frame = typeof options.requestAnimationFrame === 'function'
    ? options.requestAnimationFrame
    : (callback) => requestAnimationFrame(callback);
  const cancelFrame = typeof options.cancelAnimationFrame === 'function'
    ? options.cancelAnimationFrame
    : (handle) => cancelAnimationFrame(handle);
  const now = typeof options.now === 'function'
    ? options.now
    : () => (typeof performance !== 'undefined' ? performance.now() : Date.now());

  const connections = new Map();
  let devices = [];
  let selection = null;
  let captureTarget = null;
  let previousPressed = new Set();
  const continuousOutputs = new Map();
  let neutralGate = false;
  let lastContext = null;
  let lastStateKey = '';
  let frameHandle = null;
  let stopRuntime = null;
  let visibilitySuspended = false;
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

  function allActions() {
    return Array.from(getActions() || []);
  }

  function actionsFor(context) {
    return Array.from(getActions(context) || []).filter((action) => action
      && Array.isArray(action.contexts) && action.contexts.includes(context));
  }

  function continuousAction(action) {
    return action && action.continuous && Number.isFinite(action.continuous.neutral);
  }

  function dispatchContinuous(action, value, timestamp, immediate = false) {
    const spec = action.continuous;
    const previous = continuousOutputs.get(action.id);
    const isNeutral = value === spec.neutral;
    if (isNeutral && (!previous || previous.value === spec.neutral)) return false;
    if (!isNeutral && previous && previous.value !== spec.neutral && !immediate
        && timestamp - previous.lastSentAt < spec.cadenceMs) return false;
    if (!isTransportLive()) {
      if (isNeutral) continuousOutputs.delete(action.id);
      return false;
    }
    activate(action.id, {
      context: action.contexts[0], source: 'gamepad', value,
      neutral: isNeutral,
    });
    continuousOutputs.set(action.id, {
      value,
      neutral: spec.neutral,
      context: action.contexts[0],
      lastSentAt: timestamp,
    });
    return true;
  }

  function flushContinuous(timestamp = now()) {
    const transportLive = isTransportLive();
    for (const [actionId, output] of continuousOutputs) {
      if (output.value === output.neutral) continue;
      if (transportLive) {
        activate(actionId, {
          context: output.context, source: 'gamepad', value: output.neutral,
          neutral: true,
        });
      }
    }
    continuousOutputs.clear();
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

  function neutralize(timestamp = now()) {
    flushContinuous(timestamp);
    previousPressed.clear();
    neutralGate = !!selection;
    notify();
  }

  function select(index) {
    if (index == null || index === '') {
      flushContinuous();
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

  /**
   * Restore a persisted logical browser slot without pretending a disconnected
   * or replacement device already owns input.  A currently connected standard
   * pad is selected through the ordinary explicit path; otherwise the slot is
   * retained only for Settings/export and remains disconnected until the
   * operator selects the connection again.
   */
  function restorePreferred(index) {
    if (index == null || index === '') return select(null);
    observe(getGamepads());
    const slot = Number(index);
    if (!Number.isInteger(slot) || slot < 0) return { status: 'invalid' };
    const pad = latestSnapshot[slot];
    if (pad && pad.mapping === 'standard') return select(slot);
    flushContinuous();
    selection = { index: slot, generation: -1 };
    captureTarget = null;
    previousPressed.clear();
    neutralGate = true;
    notify();
    return { status: pad ? 'unsupported' : 'disconnected', index: slot };
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
    if (selection && selection.index === Number(index)) flushContinuous();
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
    if (selection && selection.index === Number(gamepad.index)) flushContinuous();
    record.connected = true;
    record.generation += 1;
    previousPressed.clear();
    neutralGate = !!selection;
    notify();
  }

  function continuousBindingsNeutral(gamepad) {
    for (const action of allActions()) {
      if (!continuousAction(action)) continue;
      for (const binding of action.bindings || []) {
        if (!binding || binding.type !== 'gamepad' || binding.input !== 'axis'
            || binding.direction != null) continue;
        const mapped = STANDARD_GAMEPAD_CONTROLS[binding.control];
        if (!mapped || mapped.input !== 'axis') continue;
        const value = normalizeContinuousAxis(
          (gamepad.axes || [])[mapped.index], action.tuning, action.continuous,
        );
        if (value !== action.continuous.neutral) return false;
      }
    }
    return true;
  }

  function discreteBindingsNeutral(gamepad) {
    for (const action of allActions()) {
      if (continuousAction(action)) continue;
      for (const binding of action.bindings || []) {
        if (binding && binding.type === 'gamepad'
            && gamepadBindingPressed(binding, gamepad)) return false;
      }
    }
    return true;
  }

  function allBoundInputsNeutral(gamepad) {
    if (captureTarget) {
      const targetAction = allActions().find((action) => action.id === captureTarget.actionId);
      if (firstCaptureBinding(gamepad, {
        continuous: !!continuousAction(targetAction),
      }) !== null) return false;
    }
    return discreteBindingsNeutral(gamepad) && continuousBindingsNeutral(gamepad);
  }

  function sampleContinuous(action, gamepad) {
    let selected = null;
    for (let slot = 0; slot < (action.bindings || []).length; slot++) {
      const binding = action.bindings[slot];
      if (!binding || binding.type !== 'gamepad' || binding.input !== 'axis'
          || binding.direction != null) continue;
      const mapped = STANDARD_GAMEPAD_CONTROLS[binding.control];
      if (!mapped || mapped.input !== 'axis') continue;
      const value = normalizeContinuousAxis(
        (gamepad.axes || [])[mapped.index], action.tuning, action.continuous,
      );
      const deflection = continuousDeflection(value, action.continuous);
      if (!selected || deflection > selected.deflection) {
        selected = { value, deflection, slot };
      }
    }
    return selected ? selected.value : action.continuous.neutral;
  }

  function poll(snapshot = getGamepads(), timestamp = now()) {
    observe(snapshot);
    const context = getContext() || null;
    if (context !== lastContext) {
      flushContinuous(timestamp);
      lastContext = context;
      previousPressed.clear();
      neutralGate = !!selection;
      notify();
    }
    // A background page must not silently satisfy the neutral gate and resume
    // output before the operator can see it. The visible transition re-arms
    // the gate, so a displaced axis remains inert until it is deliberately
    // returned to neutral in the foreground.
    if (visibilitySuspended) {
      previousPressed.clear();
      notify();
      return state();
    }
    const pad = selectedPad();
    if (!pad || pad.mapping !== 'standard') {
      flushContinuous(timestamp);
      previousPressed.clear();
      notify();
      return state();
    }
    if (neutralGate) {
      if (allBoundInputsNeutral(pad)) neutralGate = false;
      previousPressed.clear();
      notify();
      return state();
    }
    if (captureTarget) {
      const targetAction = allActions().find((action) => action.id === captureTarget.actionId);
      const binding = firstCaptureBinding(pad, { continuous: !!continuousAction(targetAction) });
      if (binding) {
        const target = captureTarget;
        captureTarget = null;
        neutralize(timestamp);
        onCapture(target, binding);
      }
      notify();
      return state();
    }

    const contextActions = actionsFor(context);
    const seenContinuous = new Set();
    for (const action of contextActions) {
      if (!continuousAction(action)) continue;
      seenContinuous.add(action.id);
      const value = sampleContinuous(action, pad);
      const previous = continuousOutputs.get(action.id);
      dispatchContinuous(
        action,
        value,
        timestamp,
        !previous || previous.value === action.continuous.neutral,
      );
    }
    for (const [actionId, output] of continuousOutputs) {
      if (seenContinuous.has(actionId)) continue;
      if (output.value !== output.neutral && isTransportLive()) {
        activate(actionId, {
          context: output.context, source: 'gamepad', value: output.neutral,
          neutral: true,
        });
      }
      continuousOutputs.delete(actionId);
    }

    const nextPressed = new Set();
    for (const action of contextActions) {
      if (continuousAction(action)) continue;
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
    if (stopRuntime) return stopRuntime;
    const connected = (event) => noteConnected(event && event.gamepad);
    const disconnected = (event) => noteDisconnected(event && event.gamepad && event.gamepad.index);
    const documentHidden = () => {
      const doc = win && win.document;
      return !!(doc && (doc.hidden === true || doc.visibilityState === 'hidden'));
    };
    const visibilityChanged = () => {
      const hidden = documentHidden();
      if (hidden === visibilitySuspended) return;
      visibilitySuspended = hidden;
      neutralize();
    };
    const pageHidden = () => {
      visibilitySuspended = true;
      neutralize();
    };
    if (win && win.addEventListener) {
      win.addEventListener('gamepadconnected', connected);
      win.addEventListener('gamepaddisconnected', disconnected);
      win.addEventListener('pagehide', pageHidden);
    }
    if (win && win.document && win.document.addEventListener) {
      win.document.addEventListener('visibilitychange', visibilityChanged);
    }
    visibilitySuspended = documentHidden();
    if (visibilitySuspended) neutralize();
    const tick = (timestamp) => {
      poll(undefined, timestamp);
      frameHandle = frame(tick);
    };
    poll();
    frameHandle = frame(tick);
    let stopped = false;
    stopRuntime = () => {
      if (stopped) return;
      stopped = true;
      neutralize();
      if (frameHandle !== null) cancelFrame(frameHandle);
      frameHandle = null;
      if (win && win.removeEventListener) {
        win.removeEventListener('gamepadconnected', connected);
        win.removeEventListener('gamepaddisconnected', disconnected);
        win.removeEventListener('pagehide', pageHidden);
      }
      if (win && win.document && win.document.removeEventListener) {
        win.document.removeEventListener('visibilitychange', visibilityChanged);
      }
      stopRuntime = null;
    };
    return stopRuntime;
  }

  return {
    poll, start, state, select, restorePreferred, beginCapture, endCapture, neutralize,
    noteConnected, noteDisconnected,
  };
}

if (typeof window !== 'undefined') {
  window.createGamepadInputRuntime = createGamepadInputRuntime;
}
