// Review-only workload scripts injected into a private native bundle.
// They use the normal participant and private-GM adapters, including admission
// and correlated terminal feedback. No authoritative state is written here.
// Every telemetry producer has the same explicit size/pending/HTTP failure path.
import { createEffectWitness } from './fleet-effect-witness.mjs';
import { captureDirectEffect } from './fleet-browser-recovery.mjs';
export function nativeObserverReporterScript(endpoint) {
  return `const observerEndpoint = ${JSON.stringify(endpoint)};
  const observerPending = new Set();
  let observerFailed = false;
  const observerFail = (kind, reason) => {
    if (observerFailed) return;
    observerFailed = true;
    fetch(observerEndpoint + '?event=' + encodeURIComponent(JSON.stringify({kind,value:{reason}}))).catch(() => {});
  };
  const report = (kind, value) => {
    if (observerFailed) return;
    let url;
    try { url = observerEndpoint + '?event=' + encodeURIComponent(JSON.stringify({kind,value})); }
    catch (_) { observerFail('observer-error','event-serialization'); return; }
    if (url.length > 60000) { observerFail('observer-overflow','event-size'); return; }
    if (observerPending.size >= 256) { observerFail('observer-overflow','pending-telemetry'); return; }
    const request = fetch(url);
    observerPending.add(request);
    request.then(response => {
      if (!response.ok) observerFail('observer-error','http-status-' + response.status);
    }, () => observerFail('observer-error','fetch-failed')).finally(() => observerPending.delete(request));
  };`;
}

export function paneWorkloadScript(endpoint) {
  return `(() => {
    const pane = window.__phoenixPane;
    if (!pane || !/^matrix-(captain|helm|engineering)$/.test(pane.name)) return;
    const station = pane.name.slice(7);
    ${nativeObserverReporterScript(endpoint)}
    const reportStation = (kind,value) => report(kind,{station,...value});
    let welcomed = false, assigned = false, ready = false, started = false, claimSent = false, sequence = 0;
    const original = window.__phoenixPaneApply;
    window.__phoenixPaneApply = function(json) {
      const result = original.apply(this, arguments);
      try {
        const message = JSON.parse(json), data = message.data || {};
        if (message.type === 'Welcome') { welcomed = true; reportStation('station-welcome', {}); }
        if (message.type === 'StationAssigned' && data.token === pane.token) {
          const actual = typeof data.station_id === 'string' ? data.station_id : data.station_id?.id;
          assigned = actual === station; reportStation('station-assigned', {actual,assigned});
        }
        if (message.type === 'ReadyChanged' && data.token === pane.token) { ready = !!data.ready; reportStation('station-ready', {ready}); }
        if (message.type === 'GameStarted') { started = true; reportStation('station-started', {}); }
        if (message.type === 'ActionFeedback') reportStation('station-feedback', data);
      } catch (error) { reportStation('station-error', {message:error.message}); }
      return result;
    };
    const send = (type,data) => window.phoenixLink?.send(type,data,'reliable');
    let readySent = false;
    setInterval(() => {
      if (!welcomed || !window.phoenixLink) return;
      if (!claimSent) { send('SelectStation',{station}); claimSent = true; }
      if (assigned && !readySent) { send('SetReady',{ready:true}); readySent = true; }
      if (!assigned || !ready || !started) return;
      const correlation = 'native-matrix-' + station + '-' + (++sequence);
      const payload = station === 'captain'
        ? {target:'red-alert',payload:{type:'SetRedAlert',data:{active:sequence%2===0}}}
        : station === 'helm'
          ? {target:'helm-boost',payload:{type:'SetBoost',data:{active:sequence%2===0}}}
          : {target:'power-reactor',payload:{type:'SetPowerGroupAllocation',data:{group:'weapons',level:sequence%2 ? 3 : 2}}};
      send('ControlSystemCorrelated',{correlation,...payload});
      reportStation('station-command', {correlation});
    }, 1000);
    reportStation('station-engine', {userAgent:navigator.userAgent});
  })();`;
}

export function instrumentNativeGmModule(source, endpoint, {deferReady = false} = {}) {
  const declaration = 'export function mountNativeGmWorkspace(';
  if (source.split(declaration).length !== 2) throw new Error('Native GM factory declaration changed');
  return source.replace(declaration, 'function observedNativeGmWorkspace(') + `
export function mountNativeGmWorkspace(options) {
  const workspace = observedNativeGmWorkspace(options);
  const bridge = options.bridge;
  ${nativeObserverReporterScript(endpoint)}
  let phase = 'Lobby', readyAllowed = ${JSON.stringify(!deferReady)}, readyConfirmed = false, sequence = 0, busy = false;
  let lastMetadata = '';
  const observedActions = new Set();
  let effectRequest=null;
  // The embedded engine may lack structuredClone; activity projections contain
  // only JSON data, so this local copy has the same value semantics.
  const structuredClone = value => JSON.parse(JSON.stringify(value));
  const createEffectWitness = ${createEffectWitness.toString()};
  const captureDirectEffect = ${captureDirectEffect.toString()};
  setInterval(()=>{
    if(!effectRequest)return;
    try { report('effect-witness',captureDirectEffect(effectRequest)); }
    catch(error){report('observer-error',{reason:'effect-witness: '+error.message});}
  },500);
  const tryReady = () => {
    if (readyAllowed && !readyConfirmed && phase === 'Lobby' && bridge.getOperator()?.connected) {
      bridge.setReady(true); report('gm-ready-requested',{});
    }
  };
  bridge.subscribe((channel,payload) => {
    if (channel === 'metadata') {
      phase = payload.phase;
      const metadata = {phase,local_operator_id:payload.local_operator_id,gms:payload.gms,start_policy:payload.start_policy};
      const text = JSON.stringify(metadata);
      if (text !== lastMetadata) { lastMetadata = text; report('gm-metadata',metadata); }
      if (payload.gms?.some(gm => gm.id === payload.local_operator_id && gm.ready)) readyConfirmed = true;
      tryReady();
    }
    if (channel === 'gm_activity') {
      const entries = [];
      for (const entry of payload.entries || []) {
        if (entry.detail?.type !== 'gm_action') continue;
        const data = entry.detail.data;
        const key = JSON.stringify([data.operator?.id,data.correlation,data.outcome]);
        if (observedActions.has(key)) continue;
        if (observedActions.size >= 4096) { observerFail('observer-overflow','gm-action-count'); return; }
        observedActions.add(key); entries.push(entry);
      }
      // Preserve each attributed action once; repeating a growing history in
      // a GET URL could exceed the HTTP parser before the listener saw it.
      for (const entry of entries) report('gm-activity',{entries:[entry]});
    }
    if (channel === 'gm_session') report('gm-session',{paused:payload.paused,results:payload.results,
      factions:payload.factions,journalTotal:payload.journal?.total});
  });
  setInterval(async () => {
    if (busy) return;
    busy = true;
    try {
      const response = await fetch(${JSON.stringify(endpoint + '/control')});
      const command = await response.json();
      if (command?.kind === 'ready') readyAllowed = true;
      tryReady();
      if (command?.kind === 'force-start') { bridge.forceStart(); report('gm-force-start-requested',{}); }
      if (command?.kind === 'effect-track') effectRequest=command.request;
      if (command?.kind === 'effect-observe') {
        effectRequest=command.request;
        createEffectWitness({...effectRequest,observe:true,maxDurationMs:600000,maxSamples:14000,maxBytes:32*1024*1024});
        report('effect-observer-started',{});
      }
      if (command?.kind === 'effect-apply') {
        const accepted=window.__hostApplyDirectEffect({...command.request,effect:'damage',scope:'entity',scope_id:null});
        report('effect-requested',{accepted});
      }
    } catch (error) { report('gm-control-error',{message:error.message}); }
    finally { busy = false; }
    if (phase !== 'InProgress' || !bridge.getOperator()?.connected) return;
    const correlation = 'native-matrix-gm-' + (++sequence);
    const reverse = Number((bridge.getOperator().id.match(/[0-9]+$/) || ['0'])[0]) % 2 === 1;
    const accepted = window.__hostSetFactionHostility({correlation,faction:reverse?'Alliance':'Harrow',enemy:reverse?'Harrow':'Alliance',hostile:sequence%2===0});
    report('gm-action-requested',{correlation,accepted});
  }, 2000);
  report('gm-engine',{userAgent:navigator.userAgent});
  return workspace;
}
`;
}
