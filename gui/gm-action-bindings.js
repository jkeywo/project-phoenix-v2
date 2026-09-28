/** Explicit writes offered by the shared GM desk. Presentation subscriptions
 * are deliberately absent: they carry no simulation or save authority. */
export const GM_ACTION_BINDINGS = Object.freeze({
  __hostFireGmEvent: 'fire_gm_event',
  __hostSetGmEventPaused: 'set_event_paused',
  __hostArmGmEventSkip: 'arm_gm_event_skip',
  __hostObjectiveAction: 'objective_action',
  __hostObjectiveInstanceAction: 'objective_instance_action',
  __hostTransmitComms: 'transmit_comms',
  __hostSpawnPaletteEntity: 'spawn_palette_entity',
  __hostApplyDirectEffect: 'apply_direct_effect',
  __hostSetSystemDisabled: 'set_system_disabled',
  __hostSetContactOverride: 'set_contact_override',
  __hostSetContactClassification: 'set_contact_classification',
  __hostPresentation: 'presentation',
  __hostSetContactInformation: 'set_contact_information',
  __hostDespawnEntity: 'despawn_entity',
  __hostSetNpcDoctrine: 'set_npc_doctrine',
  __hostSetNpcDoctrineChecked: 'set_npc_doctrine_checked',
  __hostSetFactionHostility: 'set_faction_hostility',
  __hostUndoGmAction: 'undo_gm_action',
  __hostRequestLiveRestore: 'request_live_restore',
  __hostSetStationPuppet: 'set_station_puppet',
  __hostIssueStationCommand: 'issue_station_command',
  __hostBackfillShipSlot: 'backfill_ship_slot',
  // These retain their specialized argument and completion contracts.
  __hostSetSessionPaused: 'set_session_paused',
  __hostGmCheckpointCreate: null,
  __hostSaveSlotCapture: null,
  __hostSaveSlotRestore: null,
});
export const GM_ACTION_NAMES = Object.freeze(Object.keys(GM_ACTION_BINDINGS));

const owners = new WeakMap();

/** Publish only declared writes. Superseded and retained functions become inert;
 * disposing an older desk never removes a newer desk's bindings. */
export function installGmActionBindings(target, adapter) {
  let current = owners.get(target);
  if (!current) { current = new Map(); owners.set(target, current); }
  const owner = {}, installed = new Map();
  let disposed = false;
  const isCurrent = name => !disposed && current.get(name)?.owner === owner;
  for (const name of GM_ACTION_NAMES) {
    const invoke = adapter[name];
    const prior = current.get(name);
    // Optional native save capability remains absent when no provider exists.
    // Claim the slot anyway so an older mount cannot keep submitting through it.
    const binding = typeof invoke === 'function'
      ? (...args) => isCurrent(name) ? invoke(...args) : false : undefined;
    current.set(name, { owner, binding });
    if (!binding) {
      if (prior?.binding && target[name] === prior.binding) delete target[name];
      continue;
    }
    installed.set(name, binding);
    target[name] = binding;
  }
  return {
    isCurrent,
    dispose() {
      if (disposed) return;
      disposed = true;
      for (const name of GM_ACTION_NAMES) {
        const binding = installed.get(name);
        if (current.get(name)?.owner === owner) current.delete(name);
        if (binding && target[name] === binding) delete target[name];
      }
    },
  };
}

/** The classic browser script supplies its existing typed preparation functions.
 * Missing routes (including native-only BackfillShipSlot) explicitly refuse. */
export function browserGmActionAdapter(prepared) {
  return Object.fromEntries(GM_ACTION_NAMES.map(name => [name, prepared?.[name] || (() => false)]));
}

/** Native Rust still owns Admission and validates each typed request. */
export function nativeGmActionAdapter({ getOperator, submitAction, createCheckpoint }) {
  const submit = (action, request) => {
    const operator = getOperator();
    if (!operator || !request || typeof request !== 'object'
        || typeof request.correlation !== 'string' || !request.correlation
        || (request.operator_id != null && request.operator_id !== operator.id)) return false;
    try { return submitAction({ ...request, action, operator_id: operator.id }) === true; }
    catch (_) { return false; }
  };
  const adapter = Object.fromEntries(Object.entries(GM_ACTION_BINDINGS)
    .filter(([, action]) => action !== null)
    .map(([name, action]) => [name, request => submit(action, request)]));
  adapter.__hostSetSessionPaused = (active, correlation) => typeof active === 'boolean'
    && submit('set_session_paused', { active, correlation });
  adapter.__hostGmCheckpointCreate = createCheckpoint;
  return adapter;
}

/** Test has no transport or save dependency to accidentally reach. */
export function refusingGmActionAdapter(refuse) {
  return Object.fromEntries(GM_ACTION_NAMES.map(name => [name, refuse]));
}
