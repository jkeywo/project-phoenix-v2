import { mountNativeGmWorkspace } from './gui/native-gm-workspace.js';

document.documentElement.classList.add('phoenix-gm-page');
const channels = ['metadata', 'gm_entity', 'gm_activity', 'gm_station', 'gm_session', 'gm_mission', 'gm_spawn', 'gm_comms', 'gm_attention', 'gm_health', 'gm_workload'];
const listeners = new Set();
channels.push('save_reply', 'save_outcomes');
let saveSequence = 0;
let saveQueue = Promise.resolve();
const savePending = new Map();
const latest = new Map();
let operator = null;
let phase = 'Lobby';
let hostLobbyUnavailable = false;
const send = value => window.phoenixNativeGmOut.send(JSON.stringify(value));
window.PhoenixInstallNativeOperatorStorage(record => window.phoenixNativeGmOperatorOut.send(record));
window.__phoenixNativeGmChannels = Object.fromEntries(channels.map(channel => [channel, json => {
  const payload = JSON.parse(json);
  if (channel === 'save_reply') {
    const pending = savePending.get(payload.id);
    if (pending) {
      savePending.delete(payload.id); clearTimeout(pending.timer);
      if (payload.error) pending.reject(new Error(payload.error)); else pending.resolve(payload.value);
    }
    return;
  }
  latest.set(channel, payload);
  if (channel === 'metadata') {
    operator = payload.gms.find(gm => gm.id === payload.local_operator_id) || null;
    phase = payload.phase;
    hostLobbyUnavailable = payload.host_lobby_unavailable === true;
  }
  for (const listener of listeners) listener(channel, payload);
}]));
window.phoenixNativeGm = {
  inspectorInterest(panels) { return send({kind:'inspector-interest', panels}) !== false; },
  consoleInterest(request) { return send({kind:'console-interest', request}) !== false; },
  saveRequest(operation, name) {
    const request = () => new Promise((resolve, reject) => {
      const id = `save-${++saveSequence}`;
      const timer = setTimeout(() => { savePending.delete(id); reject(new Error('Save service did not respond')); }, 10000);
      savePending.set(id, { resolve, reject, timer });
      send({ kind: 'save', id, operation, name: name ?? null });
    });
    const result = saveQueue.then(request); saveQueue = result.catch(() => {}); return result;
  },
  subscribe(listener) { listeners.add(listener); for (const [channel, payload] of latest) listener(channel, payload); return () => listeners.delete(listener); },
  getOperator() { return operator; },
  submitAction(request) { if (!operator?.connected) return false; return send({kind: 'action', request: JSON.stringify(request)}) !== false; },
  setReady(ready) { send({kind: 'ready', ready: !!ready}); },
  forceStart() { send({kind: 'force-start'}); },
  returnToHostLobby() {
    if (phase !== 'Lobby' || !hostLobbyUnavailable || !operator?.connected) return false;
    return send({kind: 'recovery-host-lobby'}) !== false;
  },
};
// The browser places collective readiness in its lobby overlay. The native GM
// always shows the workspace, so reuse those controls inside it.
const startControls = document.getElementById('gm-start-controls');
if (startControls) document.getElementById('gm-console')?.prepend(startControls);
mountNativeGmWorkspace({bridge: window.phoenixNativeGm, win: window, doc: document});
send({kind: 'loaded'});
