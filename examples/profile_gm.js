// Benchmark-only driver: ordinary GM adapters, authoritative admission and real iframe.
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const mark = async (stage, extra = {}) => fetch('MARKER_ORIGIN/' + encodeURIComponent(JSON.stringify({stage, ...extra})));
const latest = {};
let updates = 0;
let controlLost = false;
let controlledShip = null;
let targetShip = null;
let mountedWindow = null;
let lastReading = 0;
let lastConsoleJson = null;
let changedReadings = 0;
const counts = {minShips:Infinity, maxShips:0, minEntities:Infinity, maxEntities:0};
const invalid = new Set();
const generations = [];
let initialGeneration = null;
let recording = false;
const consoleTimes = [];
const readingIntervals = [];
const processing = {};
const summarize = values => { const sorted = values.slice().sort((a,b)=>a-b); return {count:sorted.length, p50:sorted[Math.floor(sorted.length*0.5)], p95:sorted[Math.floor(sorted.length*0.95)], max:sorted[sorted.length-1]}; };
for (const [channel, receive] of Object.entries(window.__phoenixNativeGmChannels || {})) {
  const measurements = processing[channel] = {ms:[], codeUnits:0};
  window.__phoenixNativeGmChannels[channel] = function(json) {
    const start = performance.now();
    const result = receive.apply(this, arguments);
    if (recording) { measurements.ms.push(performance.now()-start); measurements.codeUnits += json.length; }
    return result;
  };
}
const health = setInterval(() => {
  if (!recording) return;
  const shipCount = latest.gm_station?.ships?.length || 0;
  const entityCount = latest.gm_entity?.entities?.length || 0;
  counts.minShips = Math.min(counts.minShips, shipCount); counts.maxShips = Math.max(counts.maxShips, shipCount);
  counts.minEntities = Math.min(counts.minEntities, entityCount); counts.maxEntities = Math.max(counts.maxEntities, entityCount);
  if (innerWidth !== 1920 || innerHeight !== 1080) invalid.add('resized');
  if (latest.metadata?.phase !== ('PROFILE_STAGE' === 'idle' ? 'Lobby' : 'InProgress')) invalid.add('phase');
  if ('PROFILE_STAGE' !== 'idle' && latest.gm_session?.paused) invalid.add('paused');
  if (targetShip && 'PROFILE_STAGE' === 'observed') {
    const generation = {world:latest.gm_station?.presentation_generation, mount:latest.gm_station?.console_interest?.mount_generation};
    if (generation.world == null || generation.mount == null) invalid.add('missing-console-generation');
    const key = JSON.stringify(generation);
    if (initialGeneration === null) initialGeneration = key;
    if (key !== initialGeneration) invalid.add('console-generation-changed');
    generations.push({seconds:performance.now()/1000, ...generation});
    if (window.__hostGmStationState?.().selectedRow?.ship?.ship_id !== targetShip) invalid.add('selection');
    if (document.getElementById('gm-station-frame')?.contentWindow !== mountedWindow) invalid.add('console-remounted');
    if (!document.getElementById('gm-station-connection')?.textContent.includes('Live')) invalid.add('not-live');
    if (performance.now() - lastReading > 1000) invalid.add('stale-readings');
    if (document.getElementById('gm-station-toggle')?.dataset.active !== 'false') invalid.add('not-observing');
  }
}, 250);
const wait = async (predicate, label) => {
  const end = Date.now() + 45000;
  while (!predicate()) { if (Date.now() > end) throw new Error('Timeout: ' + label); await sleep(100); }
};
const measure = async stage => {
  if (stage !== 'PROFILE_STAGE') return;
  await mark(stage + '-warmup'); await sleep(40000);
  const initialUpdates = updates;
  recording = true;
  await mark(stage + '-start'); await sleep(60000);
  recording = false;
  clearInterval(health);
  await mark('health', {invalid:[...invalid], counts, changedReadings, consoleMs:summarize(consoleTimes), readingIntervalMs:summarize(readingIntervals)});
  await mark(stage + '-end', {phase:latest.metadata?.phase, updates:updates-initialUpdates, controlLost, selectedShip:window.__hostGmStationState?.().selectedRow?.ship?.ship_id, station:document.getElementById('gm-station-connection')?.textContent, active:document.getElementById('gm-station-toggle')?.dataset.active});
  // Bounded chunks keep each marker URL small, outside the measured interval.
  for (let i=0; i<generations.length; i+=20) await mark('console-generations', {samples:generations.slice(i,i+20)});
  for (const [channel,row] of Object.entries(processing)) await mark('channel', {channel, ms:summarize(row.ms),codeUnits:row.codeUnits});
  throw 'capture-complete';
};
try {
  await wait(() => window.phoenixNativeGm?.getOperator()?.connected, 'GM admission');
  window.phoenixNativeGm.subscribe((channel,payload) => {
    latest[channel] = payload;
    if (controlledShip && channel === 'gm_station' && !payload.ships.find(ship => ship.ship_id === controlledShip)?.stations.find(station => station.station_id === 'helm')?.operators.includes(window.phoenixNativeGm.getOperator().id)) {
      if (!controlLost) mark('control-lost', {ship:controlledShip, selectedShip:window.__hostGmStationState?.().selectedRow?.ship?.ship_id});
      controlLost = true;
    }
  });
  await wait(() => latest.metadata?.ship_slots?.length, 'player ship slots');
  const slot = latest.metadata.ship_slots[0].id;
  if (latest.metadata.ship_slots[0].can_backfill) window.__hostBackfillShipSlot({slot, correlation:'profile-backfill'});
  await mark('workload', {slot, viewport:[innerWidth,innerHeight], userAgent:navigator.userAgent});
  await measure('idle');
  window.phoenixNativeGm.setReady(true);
  window.phoenixNativeGm.forceStart();
  await wait(() => latest.metadata?.phase === 'InProgress', 'mission start');
  const playerShip = () => {
    const id = latest.gm_entity?.entities?.find(entity => entity.kind === 'player_ship')?.entity_id;
    return latest.gm_station?.ships?.find(ship => ship.ship_id === id && ship.stations.some(station => station.station_id === 'helm'));
  };
  await wait(playerShip, 'player-slot Helm');
  const ship = playerShip();
  targetShip = ship.ship_id;
  await mark('ship', {ship:ship.ship_id, name:ship.name, kind:'player_ship'});
  await measure('running');
  window.__hostGmConfirmationProfile.setMode('station.takeover', 'immediate');
  window.__hostGmConfirmationProfile.setMode('station.release', 'immediate');
  window.__hostGmConfirmationProfile.setMode('station.command', 'immediate');
  window.__hostGmFocusStation(ship.ship_id, 'helm');
  await wait(() => document.getElementById('gm-station-frame')?.contentWindow?.sendAction && document.getElementById('gm-station-connection')?.textContent.includes('Live'), 'live console');
  const stationWindow = document.getElementById('gm-station-frame').contentWindow;
  mountedWindow = stationWindow;
  const update = stationWindow.__updateConsole;
  stationWindow.__updateConsole = function() {
    const start = performance.now();
    if (recording && lastReading) readingIntervals.push(start-lastReading);
    lastReading = start;
    updates++;
    if (recording && arguments[1] !== lastConsoleJson) changedReadings++;
    lastConsoleJson = arguments[1];
    const result = update.apply(this, arguments);
    if (recording) consoleTimes.push(performance.now()-start);
    return result;
  };
  // Deliberately altered pictures isolate raster costs; NEVER acceptance runs.
  // Each probe restores the original picture before the next one.
  if ('PROFILE_STAGE' === 'raster') {
    await sleep(40000);
    const animationCosts = new Map();
    for (const [realm, owner] of [['desk', window], ['console', stationWindow]]) {
      const request = owner.requestAnimationFrame.bind(owner);
      owner.requestAnimationFrame = callback => request(timestamp => {
        const start = performance.now(); callback(timestamp);
        const key = realm + ':' + String(callback).slice(0,160);
        const row = animationCosts.get(key) || {count:0, ms:0}; row.count++; row.ms += performance.now()-start;
        animationCosts.set(key,row);
      });
    }
    const roots = [];
    function collect(root) {
      roots.push(root);
      for (const node of root.querySelectorAll('*')) if (node.shadowRoot) collect(node.shadowRoot);
    }
    collect(document); collect(stationWindow.document);
    for (const [probe, css] of [
      ['unchanged', ''],
      ['no-parent-backgrounds', '#viewscreen-shell, #server-shell { background:none !important; }'],
      ['no-gm-map-layout', '#gm-entity-map { display:none !important; }'],
      ['no-console-layout', '#gm-station-frame { display:none !important; }'],
    ]) {
      const styles = roots.map(root => { const style=(root.ownerDocument || root).createElement('style'); style.textContent=css; (root.head || root).appendChild(style); return style; });
      await sleep(500);
      animationCosts.clear();
      await mark('raster-start', {probe}); await sleep(16000); await mark('raster-end', {probe});
      for (const [callback, row] of animationCosts) await mark('animation-cost', {probe,callback,...row});
      styles.forEach(style=>style.remove());
    }
    throw 'capture-complete';
  }
  await measure('observed');
  await mark('observation-proof', {updates});
  if (document.getElementById('gm-station-toggle').dataset.active !== 'true') document.getElementById('gm-station-toggle').click();
  await wait(() => document.getElementById('gm-station-toggle').dataset.active === 'true', 'authoritative takeover');
  controlledShip = ship.ship_id;
  window.__hostSetSessionPaused(false, 'profile-resume');
  stationWindow.sendAction('set_helm_thrust', {value:0.25,correlation:'profile-thrust'});
  await measure('controlled');
  await mark('complete', {updates});
} catch (error) {
  if (error === 'capture-complete') await mark('complete');
  else await mark('error', {message:String(error), phase:latest.metadata?.phase, selectedShip:window.__hostGmStationState?.().selectedRow?.ship?.ship_id});
}
