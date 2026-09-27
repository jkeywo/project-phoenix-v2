#!/usr/bin/env node
// Six real Windows processes and optional real embedded Station/GM workloads.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseNativeProbeArgs, runNativeProbe } from './fleet-native-probe.mjs';

const stationNames = ['captain','helm','engineering'];
const shipIds = ['ship-1','ship-2','ship-3','ship-4'];
const gmIds = ['gm-1','gm-2'];
const peerIds = [...shipIds,...gmIds];
const has = (rows,kind,predicate = () => true) => rows?.some(row => row.kind === kind && predicate(row.value)) || false;

export function nativeMatrixOutcome(events, {digestAfter = 0} = {}) {
  const peers = Object.fromEntries(peerIds.map(id => {
    const rows = events.get(id) || [];
    const generation = rows.filter(row => row.kind === 'state' && row.value.roster_result?.accepted).at(-1)?.value.roster_result.generation;
    const adoption = rows.filter(row => row.kind === 'simulation-roster' && row.value.generation === generation).at(-1)?.value;
    return [id,{role:id.startsWith('gm')?'gm':'ship',
      admitted: has(rows,id === 'ship-1' ? 'fleet_code' : 'fleet_join_status', value => id === 'ship-1' || value.status === 'admitted'),
      roster:rows.filter(row => row.kind === 'onRoster').at(-1)?.value,adoption,
      routes:rows.filter(row => row.kind === 'onDiag' && row.value.event === 'transport').map(row => row.value),
      errors:rows.filter(row => ['fleet_fault','page-error','page-rejection','configure-error','station-error','gm-control-error','observer-overflow','observer-error'].includes(row.kind))}];
  }));
  const stations = shipIds.flatMap(peer => stationNames.map(station => {
    const rows = events.get(peer) || [];
    const requests = rows.filter(row => row.kind === 'station-command' && row.value.station === station);
    const applied = rows.filter(row => row.kind === 'station-feedback' && row.value.station === station && row.value.outcome === 'Applied'
      && requests.some(request => request.value.correlation === row.value.correlation));
    return {peer,station,assigned:has(rows,'station-assigned',value => value.station === station && value.assigned),
      ready:has(rows,'station-ready',value => value.station === station && value.ready),
      started:has(rows,'station-started',value => value.station === station),commands:requests.length,applied:new Set(applied.map(row => row.value.correlation)).size};
  }));
  const gms = gmIds.map(peer => {
    const rows = events.get(peer) || [];
    const operator = rows.filter(row => row.kind === 'gm-metadata').at(-1)?.value.local_operator_id;
    const requests = rows.filter(row => row.kind === 'gm-action-requested' && row.value.accepted);
    const applied = new Set(rows.filter(row => row.kind === 'gm-activity').flatMap(row => row.value.entries || [])
      .filter(entry => entry.detail?.type === 'gm_action' && entry.detail.data.operator?.id === operator
        && entry.detail.data.outcome === 'applied' && requests.some(request => request.value.correlation === entry.detail.data.correlation))
      .map(entry => entry.detail.data.correlation));
    return {peer,operator,requests:requests.length,applied:applied.size};
  });
  const ticks = new Map();
  let conflictingPeerDigest = false;
  for (const peer of peerIds) for (const row of events.get(peer) || []) {
    if (row.kind !== 'digest' || typeof row.value.digest !== 'string' || (digestAfter && !(Date.parse(row.at) > digestAfter))) continue;
    if (peers[peer].adoption && row.value.from !== peers[peer].adoption.local) { conflictingPeerDigest = true; continue; }
    if (!ticks.has(row.value.tick)) ticks.set(row.value.tick,new Map());
    const previous = ticks.get(row.value.tick).get(peer);
    if (previous && previous !== row.value.digest) conflictingPeerDigest = true;
    ticks.get(row.value.tick).set(peer,row.value.digest);
  }
  const commonDigests = [...ticks].filter(([,values]) => values.size === 6)
    .map(([tick,values]) => ({tick,byPeer:Object.fromEntries(values),agreed:new Set(values.values()).size === 1})).sort((a,b) => a.tick-b.tick);
  const sixPeersAdmitted = Object.values(peers).every(peer => peer.admitted && peer.roster?.participants?.length === 6 && peer.roster?.slots?.length === 4);
  const distinctAdoptedPeers = new Set(Object.values(peers).map(peer => peer.adoption?.local)).size === 6
    && Object.values(peers).every(peer => peer.adoption?.participants?.length === 6 && peer.adoption?.shipHosts?.length === 4
      && peer.adoption.participants.includes(peer.adoption.local)
      && (peer.role === 'ship' ? peer.adoption.shipHosts.includes(peer.adoption.local)
        : peer.adoption.gmHosts.includes(peer.adoption.local) && !peer.adoption.shipHosts.includes(peer.adoption.local)));
  const observedRelay = peerIds.slice(1).every(id => peers[id].routes.some(route => route.transport === 'ws-relay'));
  return {peers,stations,gms,commonDigests,conflictingPeerDigest,sixPeersAdmitted,distinctAdoptedPeers,observedRelay,
    sixPeerWorkloadPassed:sixPeersAdmitted && distinctAdoptedPeers && observedRelay && !conflictingPeerDigest && Object.values(peers).every(peer => !peer.errors.length)
      && stations.every(station => station.assigned && station.ready && station.started && station.applied >= 2)
      && gms.every(gm => gm.applied >= 2) && commonDigests.length >= 2 && commonDigests.every(row => row.agreed)};
}

export async function runNativeAdmissionMatrix(options, hooks = {}) {
  const output = path.resolve(options.out);
  if (fs.existsSync(output)) throw new Error('Matrix output already exists');
  fs.mkdirSync(output, {recursive:true});
  const events = new Map(), commands = new Map(), fleetCommands = new Map(), promises = [], runsById = new Map(), stopped = new Set();
  const processes = new Map();
  const result = {nativeEvents:{},steps:[]};
  const step = name => { result.steps.push({name,utc:new Date().toISOString()}); process.stderr.write(name+'\n'); };
  let fleetCode, failure, stop = false, stage = 'owner';
  let forceRequested = false, digestAfter = 0;
  const started = Date.now(), deadline = started + options.seconds * 1000;
  const launch = (id,role,code,extra = {}) => {
    if (runsById.has(id)) throw new Error('Duplicate native probe '+id);
    events.set(id,[]); commands.set(id,[]); fleetCommands.set(id,[]);
    result.nativeEvents[id] = events.get(id);
    const promise = runNativeProbe({...options,...extra,role,'fleet-code':code,out:path.join(output,id),shouldStop:() => stop || stopped.has(id),
      onProcess:read => processes.set(id,read),
      nextCommand:() => commands.get(id).shift(),
      nextFleetCommand:hooks.afterHealthy ? () => fleetCommands.get(id).shift() : undefined,
      onEvent:event => { if (stop || stopped.has(id)) return; events.get(id).push(event); if (['fleet_fault','page-error','page-rejection','configure-error','station-error','gm-control-error','observer-overflow','observer-error'].includes(event.kind)
        && !(extra.claim && event.kind==='fleet_fault' && event.value.reason==='slot-taken')) failure = `${id}: ${event.kind} ${JSON.stringify(event.value)}`; if (id === 'ship-1' && event.kind === 'fleet_code') fleetCode = event.value.code; },
    }).then(result => { if (!stop && !stopped.has(id)) failure = `${id} stopped before matrix verdict`; return result; })
      .catch(error => { failure = `${id}: ${error.message}`; return {failure}; });
    promises.push(promise); runsById.set(id,promise); return promise;
  };
  const waitFor = async predicate => {
    while (!(await predicate())) {
      if (failure) throw new Error(failure);
      if (Date.now() >= deadline) throw new Error(`Native matrix deadline expired during ${stage}`);
      await new Promise(resolve => setTimeout(resolve,100));
    }
  };
  try {
    launch('ship-1','ship');
    await waitFor(() => fleetCode);
    stage = 'ship admission';
    for (let i = 2; i <= 4; i++) launch(`ship-${i}`,'ship',fleetCode);
    await waitFor(() => shipIds.slice(1).every(id => has(events.get(id),'fleet_join_status',value => value.status === 'admitted')));
    stage = 'GM admission';
    for (const id of gmIds) {
      launch(id,'gm',fleetCode);
      await waitFor(() => has(events.get(id),'fleet_join_status',value => value.status === 'admitted'));
    }
    await waitFor(() => nativeMatrixOutcome(events).sixPeersAdmitted);
    if (options.workload) {
      stage = 'twelve Station readiness';
      await waitFor(() => nativeMatrixOutcome(events).stations.every(station => station.assigned && station.ready));
      stage = 'GM readiness';
      await waitFor(() => gmIds.every(id => has(events.get(id),'gm-metadata',value => value.gms?.some(gm => gm.id === value.local_operator_id && gm.ready))));
      // The native owner's protocol includes a local GM even without an
      // assigned GM monitor. A separate admitted GM may use its normal Force
      // Start control; report that choice explicitly instead of calling this
      // an automatic all-ready launch.
      const grace = Date.now() + 6000;
      while (Date.now() < grace && !nativeMatrixOutcome(events).stations.every(station => station.started)) await new Promise(resolve => setTimeout(resolve,100));
      if (!nativeMatrixOutcome(events).stations.every(station => station.started)) {
        commands.get('gm-1').push({kind:'force-start'}); forceRequested = true;
      }
      stage = 'launch and authoritative Station/GM outcomes';
      await waitFor(() => { const result = nativeMatrixOutcome(events); return result.stations.every(row => row.started && row.applied >= 2) && result.gms.every(row => row.applied >= 2); });
      digestAfter = Date.now();
      stage = 'matching native digests after authoritative action receipts';
      await waitFor(() => nativeMatrixOutcome(events,{digestAfter}).sixPeerWorkloadPassed);
      result.healthy = nativeMatrixOutcome(events,{digestAfter});
      step('six native peers healthy with two post-workload checkpoints');
      if (hooks.afterHealthy) {
        stage = 'recovery';
        await hooks.afterHealthy({result,peers:[],step,deadline,wait:waitFor,
          stopNative:async id => {
            if (!runsById.has(id) || stopped.has(id)) throw new Error('Native probe is not running: '+id);
            stopped.add(id);
            const run = await runsById.get(id);
            if (run.failure || !run.cleanupExitObserved) throw new Error('Native fault exit unverified: '+id);
            return {id,cleanupExitObserved:true,finishedAt:run.finishedAt,binarySha256:run.binary.sha256};
          },
          launchNative:(id,extra) => launch(id,id.startsWith('gm')?'gm':'ship',fleetCode,extra),
          nativeProcess:id => processes.get(id)?.(),
          commandNative:(id,command) => fleetCommands.get(id).push(command),
          commandGm:(id,command) => commands.get(id).push(command),
          evaluate:() => { throw new Error('Native hook cannot evaluate a browser page'); },
        });
      }
    }
  } catch (error) { failure = error.message; }
  finally { stop = true; }
  // Per-process event files retain teardown. Verdict evidence stops before
  // intentional owner shutdown, which naturally disconnects its members.
  const runs = await Promise.all(promises);
  const outcome = nativeMatrixOutcome(events,{digestAfter});
  const summary = {kind:options.workload ? 'native-six-peer-workload' : 'native-six-peer-admission',elapsedMs:Date.now()-started,
    ...result,recoveryPassed:hooks.afterHealthy ? !failure && (result.recovery?.replacementOutcome || result.recovery?.outcome)?.passed === true : undefined,
    failure:failure || null,lastStage:stage,forceRequested,digestAfter:digestAfter ? new Date(digestAfter).toISOString() : null,...outcome,
    sixPeerAdmissionPassed:!failure && outcome.sixPeersAdmitted && outcome.observedRelay && Object.values(outcome.peers).every(peer => !peer.errors.length) && runs.length === 6 && runs.every(run => !run.failure && run.outcome?.bootstrapObserved),
    sixPeerWorkloadPassed:!failure && outcome.sixPeerWorkloadPassed && runs.length === 6 && runs.every(run => !run.failure && run.outcome?.bootstrapObserved),
    limits:['Loopback relay only; no ordinary internet or mobile-network evidence',
      'Bounded smoke run; no performance or endurance acceptance', 'Binary source-build provenance must be verified against the recorded external build command']};
  fs.writeFileSync(path.join(output,'matrix.json'),JSON.stringify(summary,null,2));
  return summary;
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const options = parseNativeProbeArgs(process.argv.slice(2));
  const result = await runNativeAdmissionMatrix(options);
  console.log(JSON.stringify(result,null,2));
  if (result.failure || !(options.workload ? result.sixPeerWorkloadPassed : result.sixPeerAdmissionPassed)) process.exitCode = 1;
}
