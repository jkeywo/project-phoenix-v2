import { mountNativeGmWorkspace } from './gui/native-gm-workspace.js';

document.documentElement.classList.add('phoenix-gm-page');
const channels = ['metadata', 'gm_entity', 'gm_activity', 'gm_station', 'gm_session', 'gm_mission', 'gm_spawn', 'gm_comms'];
const listeners = new Set();
const latest = new Map();
let operator = null;
const send = value => window.phoenixNativeGmOut.send(JSON.stringify(value));
window.__phoenixNativeGmChannels = Object.fromEntries(channels.map(channel => [channel, json => {
  const payload = JSON.parse(json);
  latest.set(channel, payload);
  if (channel === 'metadata') operator = payload.gms.find(gm => gm.id === 'native-gm') || null;
  for (const listener of listeners) listener(channel, payload);
}]));
window.phoenixNativeGm = {
  subscribe(listener) { listeners.add(listener); for (const [channel, payload] of latest) listener(channel, payload); return () => listeners.delete(listener); },
  getOperator() { return operator; },
  submitAction(request) { if (!operator?.connected) return false; return send({kind: 'action', request: JSON.stringify(request)}) !== false; },
  setReady(ready) { send({kind: 'ready', ready: !!ready}); },
  forceStart() { send({kind: 'force-start'}); },
};
// The browser places collective readiness in its lobby overlay. The native GM
// always shows the workspace, so reuse those controls inside it.
const startControls = document.getElementById('gm-start-controls');
if (startControls) document.getElementById('gm-console')?.prepend(startControls);
mountNativeGmWorkspace({bridge: window.phoenixNativeGm, win: window, doc: document});
send({kind: 'loaded'});
