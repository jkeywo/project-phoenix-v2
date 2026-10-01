/** Shared construction and emission for Station semantic action adapters. */
export function keyboard(code, modifiers = {}) {
  return Object.freeze({
    type: 'keyboard', code,
    ctrlKey: !!modifiers.ctrlKey,
    shiftKey: !!modifiers.shiftKey,
    altKey: !!modifiers.altKey,
    metaKey: !!modifiers.metaKey,
  });
}

export function gamepad(input, control, options = {}) {
  return Object.freeze({ type: 'gamepad', input, control, ...options });
}

export const button = (control) => gamepad('button', control);
export const dpad = (control) => gamepad('dpad', control);
export const axis = (control, direction) => gamepad('axis', control, { direction, threshold: 0.5 });

/** Author shipped metadata; validation still belongs to the registry. */
export function defineStationAction({
  id, contexts, labelKey, accessibilityKey = labelKey, bindings,
  feedback = 'authoritative', ...behavior
}) {
  return Object.freeze({
    id,
    contexts: Object.freeze([...contexts]),
    labelId: `semantic_action.${labelKey}.label`,
    accessibilityLabelId: `semantic_action.${accessibilityKey}.accessibility`,
    ...(feedback === 'local' ? { feedback: 'local' }
      : feedback === 'authoritative' ? { authoritativeFeedback: true } : {}),
    ...behavior,
    ...(behavior.continuous ? { continuous: Object.freeze({ ...behavior.continuous }) } : {}),
    ...(behavior.tuning ? { tuning: Object.freeze({ ...behavior.tuning }) } : {}),
    bindings: Object.freeze(bindings.map((binding) => binding && Object.freeze({ ...binding }))),
  });
}

/** Emit an authoritative action without taking over the adapter's eligibility checks. */
export function createCorrelatedActionSender(sendAction) {
  return (actionId, correlation, inputMs, name, payload = {}) => {
    if (typeof sendAction !== 'function' || typeof correlation !== 'string' || !correlation) return false;
    sendAction(name, { ...payload, correlation, semantic_action: actionId, __input_ms: inputMs });
    return true;
  };
}
