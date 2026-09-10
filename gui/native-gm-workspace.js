/** Native host-local transport adapter for the shared full GM workspace. */
import './strings-boot.js';
import { mountGmWorkspace } from './gm-workspace.js';
import { createHostChannel } from './host-channel.js';
import { t, has, localiseTree, applyToDom } from './strings.js';

// These are the existing GM verbs, not a native command vocabulary. Rust binds
// operator identity and admits the same typed request used by browser GMs.
const ACTIONS = Object.freeze({
  __hostFireGmEvent: 'fire_gm_event',
  __hostSetGmEventPaused: 'set_event_paused',
  __hostArmGmEventSkip: 'arm_gm_event_skip',
  __hostObjectiveAction: 'objective_action',
  __hostTransmitComms: 'transmit_comms',
  __hostSpawnPaletteEntity: 'spawn_palette_entity',
  __hostApplyDirectEffect: 'apply_direct_effect',
  __hostSetSystemDisabled: 'set_system_disabled',
  __hostSetContactOverride: 'set_contact_override',
  __hostDespawnEntity: 'despawn_entity',
  __hostSetNpcDoctrine: 'set_npc_doctrine',
  __hostSetFactionHostility: 'set_faction_hostility',
  __hostUndoGmAction: 'undo_gm_action',
  __hostSetStationPuppet: 'set_station_puppet',
  __hostIssueStationCommand: 'issue_station_command',
});

export function mountNativeGmWorkspace({ bridge, win = window, doc = win.document }) {
  // The reused host markup includes launch overlays that its browser boot
  // normally dismisses. That boot never runs on this private native surface.
  // World selection belongs to the viewscreen, not the GM's Ready controls.
  for (const id of ['landing-panel', 'scenario-panel', 'wasm-spinner']) {
    doc.getElementById(id)?.remove();
  }
  let metadata = { phase: 'Lobby', gms: [] };
  let disposed = false;
  let lastStartResult = null;
  const getOperator = () => {
    const operator = bridge.getOperator();
    return operator && operator.connected !== false && typeof operator.id === 'string'
      ? operator : null;
  };
  win.__hostLocalGm = getOperator;
  win.__hostGmName = id => metadata.gms.find(row => row.id === id)?.name || id;
  const submit = (action, request) => {
    const operator = getOperator();
    if (disposed || !operator || !request || typeof request !== 'object'
        || typeof request.correlation !== 'string' || !request.correlation
        || (request.operator_id != null && request.operator_id !== operator.id)) return false;
    try {
      return bridge.submitAction({ ...request, action, operator_id: operator.id }) === true;
    } catch (_) {
      return false;
    }
  };
  for (const [callback, action] of Object.entries(ACTIONS)) {
    win[callback] = request => submit(action, request);
  }
  win.__hostSetSessionPaused = (active, correlation) => typeof active === 'boolean'
    && submit('set_session_paused', { active, correlation });

  applyToDom(doc);
  const workspace = mountGmWorkspace({ win, doc });
  const dispatch = createHostChannel({
    handlers: workspace.handlers,
    strings: { t, has, localiseTree },
  });
  const readyButton = doc.getElementById('gm-ready-btn');
  const forceButton = doc.getElementById('gm-force-start-btn');
  const result = doc.getElementById('gm-start-result');
  const recoveryButton = doc.createElement('button');
  recoveryButton.id = 'gm-return-to-host-lobby';
  recoveryButton.type = 'button';
  recoveryButton.hidden = true;
  recoveryButton.textContent = t('server.gm.return_to_host_lobby');
  doc.getElementById('gm-start-controls')?.append(recoveryButton);
  let recoveryPending = false;
  const inLobby = () => metadata.phase === 'Lobby';
  function renderStart() {
    const operator = getOperator();
    const recoveryAvailable = inLobby() && metadata.host_lobby_unavailable === true;
    recoveryButton.hidden = !recoveryAvailable;
    if (!recoveryAvailable) recoveryPending = false;
    recoveryButton.disabled = !operator || recoveryPending || !recoveryAvailable;
    doc.getElementById('gm-start-controls')?.setAttribute('aria-hidden', operator ? 'false' : 'true');
    if (readyButton) {
      readyButton.textContent = t(operator?.ready ? 'server.gm.start.unready' : 'server.gm.start.ready');
      readyButton.disabled = !operator || !inLobby();
    }
    if (forceButton) {
      forceButton.textContent = t('server.gm.start.force');
      forceButton.disabled = !operator || !inLobby();
    }
    const policy = doc.getElementById('gm-start-policy');
    if (policy) policy.textContent = metadata.start_policy
      ? t('server.gm.start.summary', {
          ready: metadata.start_policy.ready_total,
          connected: metadata.start_policy.connected_total,
        }) : t('server.gm.start.waiting_policy');
    const start = metadata.start_result;
    if (result && start?.grant_id && start.grant_id !== lastStartResult) {
      lastStartResult = start.grant_id;
      const key = start.status === 'applied' ? 'server.gm.start.force_applied'
        : start.status === 'no-op' ? 'server.gm.start.already_started'
        : start.reason === 'validation-failed' ? 'server.gm.start.validation_failed'
        : 'server.gm.start.force_refused';
      result.textContent = t(key, { name: win.__hostGmName(start.operator_id) });
    }
  }
  function setReady() {
    const operator = getOperator();
    if (!disposed && operator && inLobby()) bridge.setReady(!operator.ready);
  }
  function forceStart() {
    if (disposed || !getOperator() || !inLobby()) return;
    const accepted = bridge.forceStart();
    if (result) result.textContent = accepted === false ? t('server.gm.start.force_refused') : '';
  }
  function returnToHostLobby() {
    if (disposed || !getOperator() || !inLobby() || metadata.host_lobby_unavailable !== true || recoveryPending) return;
    recoveryPending = bridge.returnToHostLobby?.() === true;
    renderStart();
  }
  readyButton?.addEventListener('click', setReady);
  forceButton?.addEventListener('click', forceStart);
  recoveryButton.addEventListener('click', returnToHostLobby);
  const unsubscribe = bridge.subscribe((channel, payload) => {
    if (disposed) return;
    if (channel === 'metadata') {
      let value = payload;
      if (typeof value === 'string') {
        try { value = JSON.parse(value); } catch (_) { return; }
      }
      if (!value || !Array.isArray(value.gms) || typeof value.phase !== 'string') return;
      const previousPhase = metadata.phase;
      metadata = value;
      win.__hostGmShellMetadata?.(metadata);
      if (metadata.phase === 'Lobby' && previousPhase !== 'Lobby') workspace.reset();
      if (Array.isArray(metadata.role_presets)) win.__hostGmRolePresetsSetAvailable(metadata.role_presets);
      workspace.refreshAdmission();
      renderStart();
    } else if (Object.prototype.hasOwnProperty.call(workspace.handlers, channel)) {
      dispatch(channel, payload);
    }
  });
  renderStart();
  return {
    workspace,
    dispose() {
      disposed = true;
      if (typeof unsubscribe === 'function') unsubscribe();
      readyButton?.removeEventListener('click', setReady);
      forceButton?.removeEventListener('click', forceStart);
      recoveryButton.removeEventListener('click', returnToHostLobby);
      recoveryButton.remove();
      workspace.reset();
      workspace.dispose();
    },
  };
}
