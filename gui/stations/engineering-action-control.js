/** Visible Engineering-family controls share their console's semantic owner. */
export function activateEngineeringAction(owner, actionId, detail, legacyAction) {
  const activate = typeof window !== 'undefined' && window.activateSemanticAction;
  if (typeof activate === 'function') {
    return activate(actionId, { source: 'control', detail });
  }
  if (owner && typeof owner.sendAction === 'function') {
    owner.sendAction(legacyAction, detail);
  }
  return false;
}
