/**
 * gui/gamepad-input.js — one explicitly-owned standard gamepad.
 *
 * Bindings name portable controls from the W3C "standard" mapping. Hardware
 * ids describe the preferred device, while an ephemeral generation prevents a
 * replacement in the same browser slot inheriting a connection's ownership.
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
  // One signed logical axis preserves Helm's shipped bumper pair: LB is -1,
  // RB is +1 and pressing both cancels to neutral. Continuous binding capture
  // may therefore learn the pair from either shoulder without treating two
  // required directions as two unrelated action alternatives.
  'shoulder-pair': Object.freeze({ input: 'axis', negativeIndex: 4, positiveIndex: 5,
    members: Object.freeze(['left-shoulder', 'right-shoulder']),
    labelId: 'input.gamepad.shoulder_pair' }),
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
    const controlsOverlap = a.control === b.control
      || (STANDARD_GAMEPAD_CONTROLS[a.control]?.members || []).includes(b.control)
      || (STANDARD_GAMEPAD_CONTROLS[b.control]?.members || []).includes(a.control);
    if (!controlsOverlap) return false;
    if (a.input === 'axis' && b.input === 'axis'
        && (a.direction == null || b.direction == null)) return true;
    // A composite signed axis owns each of its constituent buttons, so the
    // defaults cannot silently assign LB/RB to another concurrent action.
    if (a.input !== b.input) return true;
    return a.input === b.input
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

function controlAxis(gamepad, mapped) {
  if (Number.isInteger(mapped?.negativeIndex) && Number.isInteger(mapped?.positiveIndex)) {
    const negative = pressedButton((gamepad.buttons || [])[mapped.negativeIndex]) ? -1 : 0;
    const positive = pressedButton((gamepad.buttons || [])[mapped.positiveIndex]) ? 1 : 0;
    return negative + positive;
  }
  return Number((gamepad.axes || [])[mapped?.index]) || 0;
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
  const axis = controlAxis(gamepad, mapped);
  return binding.direction === 'negative'
    ? axis <= -binding.threshold : axis >= binding.threshold;
}

/** Connected browser slots for Settings. Hardware names are intentionally absent. */
export function enumerateGamepads(snapshot) {
  return Array.from(snapshot || [])
    .filter(Boolean)
    .map((pad) => ({ index: Number(pad.index), supported: pad.mapping === 'standard',
      ...(pad.assignedTo ? { assignedTo: pad.assignedTo } : {}),
      ...(pad.available === false ? { available: false } : {}),
    }))
    .filter((pad) => Number.isInteger(pad.index) && pad.index >= 0)
    .sort((a, b) => a.index - b.index);
}

export function gamepadDevicePreference(pad) {
  return pad && typeof pad.id === 'string' && pad.id.trim() && pad.mapping === 'standard'
    ? { id: pad.id, mapping: 'standard' } : null;
}

export function usableGamepadControls(pad) {
  if (!pad || pad.mapping !== 'standard') return [];
  return CONTROL_ENTRIES.filter(([, control]) => control.negativeIndex != null
    ? pad.buttons?.[control.negativeIndex] != null && pad.buttons?.[control.positiveIndex] != null
    : control.input === 'axis' ? Number.isFinite(pad.axes?.[control.index])
      : pad.buttons?.[control.index] != null).map(([id]) => id);
}

/** Identical controllers have no portable serial number: never guess between them. */
export function matchPreferredGamepad(snapshot, preference) {
  if (!preference?.id || preference.mapping !== 'standard') return { status: 'none' };
  const matches = Array.from(snapshot || []).filter((pad) => pad
    && pad.id === preference.id && pad.mapping === preference.mapping);
  if (matches.length > 1) return { status: 'ambiguous' };
  if (!matches.length) return { status: 'disconnected' };
  const pad = matches[0];
  return pad.available === false ? { status: 'assigned' }
    : { status: 'matched', index: Number(pad.index) };
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
    const value = controlAxis(gamepad, mapped);
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
  let preferredDevice = null;
  let restoreStatus = 'none';
  let captureTarget = null;
  let previousPressed = new Set();
  const continuousOutputs = new Map();
  const heldDiscrete = new Map();
  let neutralGate = false;
  let lastContext = null;
  let lastActionCatalogueSignature = null;
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
      const identity = latestSnapshot[device.index]?.id || '';
      if (!record.connected || record.identity !== identity) {
        record.connected = true;
        record.generation += 1;
        record.identity = identity;
      }
    }
    for (const [index, record] of connections) {
      if (record.connected && !present.has(index)) record.connected = false;
    }
    devices = enumerateGamepads(latestSnapshot);
    if (preferredDevice && !selectedPad()) {
      const match = matchPreferredGamepad(latestSnapshot, preferredDevice);
      restoreStatus = match.status;
      if (match.status === 'ambiguous') selection = null;
      if (match.status === 'matched') {
        flushContinuous();
        flushDiscreteHolds();
        selection = { index: match.index, generation: connection(match.index).generation };
        previousPressed.clear();
        neutralGate = true;
        options.requestSelection?.(match.index);
      }
    }
  }

  function selectedPad() {
    if (!selection) return null;
    const record = connection(selection.index);
    if (!record.connected || record.generation !== selection.generation) return null;
    const pad = latestSnapshot[selection.index];
    return pad && Number(pad.index) === selection.index && pad.available !== false ? pad : null;
  }

  function readActions(context) {
    const actions = getActions(context);
    return actions == null ? null : Array.from(actions);
  }

  function actionsFor(context, actions) {
    return Array.from(actions || []).filter((action) => action
      && Array.isArray(action.contexts) && action.contexts.includes(context));
  }

  // The identity and live gamepad shape are enough to decide whether a newly
  // loaded/remapped catalogue can make a held input actionable. Functions and
  // presentation strings are deliberately excluded so harmless render changes
  // cannot re-arm ownership.
  function catalogueSignatureFor(actions) {
    if (actions == null) return null;
    return JSON.stringify(actions.map((action) => [
      action && action.id,
      action && action.contexts,
      action && action.bindings,
      action && action.continuous,
      action && action.tuning,
    ]));
  }

  function continuousAction(action) {
    return action && action.continuous && Number.isFinite(action.continuous.neutral);
  }

  function dispatchContinuous(action, context, value, timestamp, immediate = false) {
    const spec = action.continuous;
    const previous = continuousOutputs.get(action.id);
    const isNeutral = value === spec.neutral;
    if (isNeutral && (!previous || previous.value === spec.neutral)) return false;
    if (!isNeutral && previous && previous.value !== spec.neutral && !immediate
        && timestamp - previous.lastSentAt < spec.cadenceMs) return false;
    if (!isTransportLive()) {
      continuousOutputs.delete(action.id);
      return false;
    }
    // A composite console can expose the same semantic action in more than
    // one context.  The active caller-selected context owns the first live
    // output; its neutral must return through that same context even if the
    // parent has selected another subcontext in the meantime.
    const outputContext = previous?.context ?? context;
    activate(action.id, {
      context: outputContext, source: 'gamepad', value,
      neutral: isNeutral,
    });
    continuousOutputs.set(action.id, {
      value,
      neutral: spec.neutral,
      context: outputContext,
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

  function flushDiscreteHolds() {
    const transportLive = isTransportLive();
    for (const held of heldDiscrete.values()) {
      if (transportLive) {
        activate(held.actionId, {
          context: held.context,
          source: 'gamepad',
          binding: { ...held.binding },
          pressed: false,
        });
      }
    }
    heldDiscrete.clear();
  }

  function status() {
    if (restoreStatus === 'ambiguous' && !selectedPad()) return 'ambiguous';
    if (restoreStatus === 'assigned' && !selectedPad()) return 'assigned';
    if (!selection) return preferredDevice ? 'disconnected' : 'none';
    const pad = selectedPad();
    if (!pad) return 'disconnected';
    if (pad.mapping !== 'standard') return 'unsupported';
    if (pad.nativeOwned === false) return 'pending';
    if (neutralGate) return 'neutral';
    return 'ready';
  }

  function state() {
    return {
      devices: devices.map((device) => ({ ...device })),
      selectedIndex: selection ? selection.index : null,
      preferredDevice,
      connected: selectedPad()?.mapping === 'standard' && selectedPad().nativeOwned !== false,
      controls: usableGamepadControls(selectedPad()),
      context: lastContext,
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
    flushDiscreteHolds();
    previousPressed.clear();
    neutralGate = !!selection;
    notify();
  }

  function select(index) {
    if (index == null || index === '') {
      flushContinuous();
      flushDiscreteHolds();
      selection = null;
      preferredDevice = null;
      restoreStatus = 'none';
      options.requestSelection?.(null);
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
    if (pad.available === false) return { status: 'assigned' };
    selection = { index: slot, generation: record.generation };
    preferredDevice = gamepadDevicePreference(pad);
    restoreStatus = 'none';
    options.requestSelection?.(slot);
    captureTarget = null;
    neutralize();
    return { status: 'selected', index: slot };
  }

  function restoreDevice(preference) {
    select(null);
    preferredDevice = preference?.id && preference.mapping === 'standard' ? { ...preference } : null;
    observe(getGamepads());
    notify();
    return state();
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
    flushDiscreteHolds();
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
    if (selection && selection.index === Number(index)) {
      flushContinuous();
      flushDiscreteHolds();
    }
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
    if (selection && selection.index === Number(gamepad.index)) {
      flushContinuous();
      flushDiscreteHolds();
    }
    record.connected = true;
    record.generation += 1;
    previousPressed.clear();
    neutralGate = !!selection;
    notify();
  }

  function continuousBindingsNeutral(gamepad, actions) {
    for (const action of actions || []) {
      if (!continuousAction(action)) continue;
      for (const binding of action.bindings || []) {
        if (!binding || binding.type !== 'gamepad' || binding.input !== 'axis'
            || binding.direction != null) continue;
        const mapped = STANDARD_GAMEPAD_CONTROLS[binding.control];
        if (!mapped || mapped.input !== 'axis') continue;
        const value = normalizeContinuousAxis(
          controlAxis(gamepad, mapped), action.tuning, action.continuous,
        );
        if (value !== action.continuous.neutral) return false;
      }
    }
    return true;
  }

  function discreteBindingsNeutral(gamepad, actions) {
    for (const action of actions || []) {
      if (continuousAction(action)) continue;
      for (const binding of action.bindings || []) {
        if (binding && binding.type === 'gamepad'
            && gamepadBindingPressed(binding, gamepad)) return false;
      }
    }
    return true;
  }

  function allBoundInputsNeutral(gamepad, actions) {
    if (captureTarget) {
      const targetAction = (actions || [])
        .find((action) => action.id === captureTarget.actionId);
      if (firstCaptureBinding(gamepad, {
        continuous: !!continuousAction(targetAction),
      }) !== null) return false;
    }
    return discreteBindingsNeutral(gamepad, actions)
      && continuousBindingsNeutral(gamepad, actions);
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
        controlAxis(gamepad, mapped), action.tuning, action.continuous,
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
    const actionCatalogue = readActions(context);
    const actionCatalogueSignature = catalogueSignatureFor(actionCatalogue);
    if (context !== lastContext) {
      flushContinuous(timestamp);
      flushDiscreteHolds();
      lastContext = context;
      lastActionCatalogueSignature = actionCatalogueSignature;
      previousPressed.clear();
      neutralGate = !!selection;
      notify();
    } else if (actionCatalogueSignature !== lastActionCatalogueSignature) {
      // An iframe finishing load (or a live remap) can introduce a binding that
      // was already held while the previous catalogue was unavailable. Treat
      // that as a new ownership boundary and require every newly visible input
      // to return to neutral before it can dispatch.
      flushContinuous(timestamp);
      flushDiscreteHolds();
      lastActionCatalogueSignature = actionCatalogueSignature;
      previousPressed.clear();
      neutralGate = !!selection;
      notify();
    }
    if (!isTransportLive()) {
      // A transport outage cannot carry a release. Drop ownership now and
      // re-arm through neutral so neither a hold nor an axis replays a stale
      // release when transport returns.
      flushContinuous(timestamp);
      flushDiscreteHolds();
      previousPressed.clear();
      neutralGate = !!selection;
      notify();
      return state();
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
    if (!pad || pad.mapping !== 'standard' || pad.nativeOwned === false) {
      flushContinuous(timestamp);
      flushDiscreteHolds();
      previousPressed.clear();
      notify();
      return state();
    }
    if (actionCatalogue == null) {
      // Missing/not-yet-loaded iframe capability is not an empty, neutral
      // surface. Keep ownership gated until the iframe can expose the bindings
      // that must be sampled.
      previousPressed.clear();
      neutralGate = !!selection;
      notify();
      return state();
    }
    if (neutralGate) {
      if (allBoundInputsNeutral(pad, actionCatalogue)) neutralGate = false;
      previousPressed.clear();
      notify();
      return state();
    }
    if (captureTarget) {
      const targetAction = actionCatalogue
        .find((action) => action.id === captureTarget.actionId);
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

    const contextActions = actionsFor(context, actionCatalogue);
    const seenContinuous = new Set();
    for (const action of contextActions) {
      if (!continuousAction(action)) continue;
      seenContinuous.add(action.id);
      const value = sampleContinuous(action, pad);
      const previous = continuousOutputs.get(action.id);
      dispatchContinuous(
        action,
        context,
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
          const result = activate(action.id, {
            context, source: 'gamepad', binding: { ...binding }, pressed: true,
          });
          if (action.hold && (result === true || result?.handled === true)) {
            heldDiscrete.set(key, { actionId: action.id, context, binding: { ...binding } });
          }
        }
      }
    }
    for (const [key, held] of heldDiscrete) {
      if (nextPressed.has(key)) continue;
      if (isTransportLive()) {
        activate(held.actionId, {
          context: held.context,
          source: 'gamepad',
          binding: { ...held.binding },
          pressed: false,
        });
      }
      heldDiscrete.delete(key);
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
    poll, start, state, select, restorePreferred, restoreDevice, beginCapture, endCapture, neutralize,
    noteConnected, noteDisconnected,
  };
}

if (typeof window !== 'undefined') {
  window.createGamepadInputRuntime = createGamepadInputRuntime;
}
