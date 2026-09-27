#!/usr/bin/env node
// #1534 actual browser faults after the #1530 healthy six-peer gate.
import path from 'node:path';
import { readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { main as browserMatrix } from './fleet-browser-matrix.mjs';
import { replacementHook, REPLACEMENT_PHASE_SECONDS } from './fleet-browser-replacement.mjs';
import { createEffectWitness } from './fleet-effect-witness.mjs';

export const FAILURES = ['ship', 'leader', 'gm'];
export function failureOutcome(evidence) {
  const survivors = evidence.after || [];
  const expected = evidence.before?.filter(peer => peer.label !== evidence.victim.label) || [];
  const ticks = new Map();
  let invalidDigest = false;
  for (const peer of survivors) for (const digest of peer.frames?.filter(frame => frame.t === 'digest') || []) {
    if (digest.d.from !== peer.mesh?.slot || digest.d.tick <= evidence.atTick) continue;
    if (typeof digest.d.digest !== 'string' || !/^[0-9a-f]{16}$/.test(digest.d.digest)) { invalidDigest = true; continue; }
    if (!ticks.has(digest.d.tick)) ticks.set(digest.d.tick, new Map());
    const prior = ticks.get(digest.d.tick).get(peer.label);
    if (prior !== undefined && prior !== digest.d.digest) invalidDigest = true;
    ticks.get(digest.d.tick).set(peer.label, digest.d.digest);
  }
  const commonDigests = [...ticks].filter(([, peers]) => peers.size === expected.length)
    .map(([tick, peers]) => ({ tick, byPeer: Object.fromEntries(peers), agreed: new Set(peers.values()).size === 1 }));
  const resumed = expected.length === 5 && survivors.length === 5 && survivors.every(peer =>
    expected.some(before => before.label === peer.label && before.mesh.slot === peer.mesh?.slot)
    && peer.phase === 'InProgress' && peer.mesh?.tick > evidence.atTick + 30
    && peer.mesh?.peers.length === 4 && !peer.mesh.peers.includes(evidence.victim.slot));
  const applied = survivors.map(peer => peer.mesh?.recovery?.losses?.find(row => row.slot === evidence.victim.slot));
  const lossApplied = survivors.length === 5 && applied.every(Boolean)
    && new Set(applied.map(row => row.tick)).size === 1;
  const backfillApplicable = evidence.before?.some(peer => peer.mesh?.recovery?.ships?.some(ship => ship.slot === evidence.victim.slot)) || false;
  const backfillVerified = lossApplied && (!backfillApplicable || survivors.every(peer =>
    peer.mesh.recovery.ships.some(ship => ship.slot === evidence.victim.slot && ship.crewed === false)));
  const ownerCommitted = evidence.failure !== 'leader' || survivors.every(peer =>
    peer.continuation?.status === 'committed' && peer.continuation.loss_tick === applied[0]?.tick);
  return { resumed, commonDigests, survivorAgreement: resumed && !invalidDigest && commonDigests.length >= 2 && commonDigests.every(row => row.agreed),
    lossApplied, lossTick: lossApplied ? applied[0].tick : null, backfillApplicable, backfillVerified, ownerCommitted };
}

export function observeRecovery() {
  const evidence = window.__recoveryEvidence = { frames: [], commands: [], overflow: false };
  const take = window.wasm_take_mesh_frames;
  if (typeof take !== 'function') throw new Error('Missing production mesh egress');
  window.wasm_take_mesh_frames = function(...args) {
    const raw = take(...args);
    for (const frame of JSON.parse(raw)) {
      if (frame.t === 'tick') for (const command of frame.d.commands || []) {
        if (evidence.commands.length >= 5000) evidence.overflow = true;
        else evidence.commands.push({ from: frame.d.from, origin: command.origin, seq: command.seq,
          tick: command.tick, ship: command.ship, target: command.target, type: command.payload?.type });
      }
      if (!['digest', 'host-loss', 'slot-claim', 'recovery-ready'].includes(frame.t)) continue;
      if (evidence.frames.length >= 2000) evidence.overflow = true;
      else evidence.frames.push(frame);
    }
    return raw;
  };
}
export function failureHook(failure, { faultSeconds = 60 } = {}) {
  if (!FAILURES.includes(failure)) throw new Error(`Unsupported failure ${failure}`);
  return async ({result, ships, gms, step}) => {
    const peers = [...ships.map((page,index) => ({page,label:`ship-${index+1}`})), ...gms.map((page,index) => ({page,label:`gm-${index+1}`}))];
    const read = async peer => ({label:peer.label,...await peer.page.evaluate(() => ({mesh:window.__hostMeshStatus(),phase:window.__saveSlotsPhase,continuation:window.wasm_fleet_continuation_status?JSON.parse(window.wasm_fleet_continuation_status()):null,...window.__recoveryEvidence}))});
    await Promise.all(peers.map(peer => peer.page.evaluate(observeRecovery)));
    const before = await Promise.all(peers.map(read));
    const victim = peers.find(peer => peer.label === ({ship:'ship-2',leader:'ship-1',gm:'gm-1'})[failure]);
    const victimState = before.find(peer => peer.label === victim.label);
    const evidence = result.recovery = {failure, before, victim:{label:victim.label,slot:victimState.mesh.slot}, atTick:Math.max(...before.map(peer=>peer.mesh.tick)), samples:[], startedUtc:new Date().toISOString()};
    await victim.page.close();
    step(`closed ${failure} ${victim.label}, slot ${evidence.victim.slot}`);
    const survivors = peers.filter(peer => peer !== victim);
    const deadline = Date.now() + faultSeconds * 1000;
    do {
      await new Promise(resolve=>setTimeout(resolve,500));
      evidence.after = await Promise.all(survivors.map(read));
      evidence.samples.push(evidence.after.map(({label,mesh})=>({label,mesh})));
      evidence.outcome = failureOutcome(evidence);
      if (evidence.after.some(peer=>peer.overflow)) throw new Error('Recovery observer overflow');
      if (evidence.outcome.survivorAgreement && evidence.outcome.lossApplied && evidence.outcome.backfillVerified && evidence.outcome.ownerCommitted) return;
    } while (Date.now() < deadline);
    throw new Error(`${failure}: missing matching post-loss digests, agreed applied loss/Backfill, or owner commit across all five survivors within ${faultSeconds}s`);
  };
}
// Only the actual direct-damage applier emits this activity fact. The durable
// action row supplies attribution; it is not itself proof that the reducer ran.
export function captureDirectEffect({entity,correlation}) {
  const activity=window.__hostGmActivityState?.();
  const journal=window.__hostGmJournalState?.();
  return {capacity:activity?.capacity,oldestTick:activity?.entries?.[0]?.tick,
    continuous:window.__recoveryEffectWitness?.read(),
    events:(activity?.entries || []).filter(row=>row.category==='damage'
      && row.detail?.type==='damage' && row.detail.data?.weapon==='gm.direct'
      && row.links?.some(link=>link.role==='victim' && link.entity?.entity_id===entity)),
    journal:(journal?.entries || []).filter(row=>row.correlation===correlation)
      .map(({correlation,action_kind,tick,sequence,outcome})=>({correlation,action_kind,tick,sequence,outcome}))};
}
export function directEffectOutcome(evidence) {
  const baseline=evidence.effectBefore || [],after=evidence.effectAfter || [];
  const request=evidence.effectRequest;
  const valid=baseline.length===2 && after.length===2 && request?.amount_milli_hp===5000
    && new Set(baseline.map(row=>row.label)).size===2 && baseline.every(row=>{
      const event=row.events?.[0],entry=row.journal?.[0];
      return row.events.length===1 && row.journal.length===1 && Number.isSafeInteger(event?.tick)
        && Number.isSafeInteger(row.oldestTick) && row.oldestTick<event.tick
        && event.category==='damage' && event.detail?.type==='damage'
        && event.detail.data?.weapon==='gm.direct' && event.detail.data.amount===5
        && Math.round(event.detail.data.hull_damage*1000)===5000
        && event.links?.some(link=>link.role==='victim' && link.entity?.entity_id===request.entity)
        && entry?.correlation===request.correlation && entry.action_kind==='direct-effect'
        && entry.outcome==='applied' && Number.isSafeInteger(entry.tick) && entry.tick>0
        && Number.isSafeInteger(entry.sequence) && entry.sequence>0;
    });
  if(!valid || new Set(baseline.map(row=>row.events[0].tick)).size!==1
    || new Set(baseline.map(row=>`${row.journal[0].tick}:${row.journal[0].sequence}`)).size!==1)return false;
  const snapshots=[...(evidence.effectSamples || []),after];
  return snapshots.every(rows=>rows.length===2 && baseline.every(before=>{
    const current=rows.find(row=>row.label===before.label);
    if (!current || JSON.stringify(current.journal)!==JSON.stringify(before.journal)) return false;
    if (evidence.effectWitnessRequired) {
      const initial=before.continuous, observed=current.continuous;
      return initial && observed && !initial.error && !observed.error
        && Number.isSafeInteger(initial.samples) && initial.samples>0
        && Number.isSafeInteger(observed.samples) && observed.samples>=initial.samples
        && observed.throughTick>=before.events[0].tick
        && JSON.stringify(initial.events)===JSON.stringify(before.events)
        && JSON.stringify(observed.events)===JSON.stringify(before.events);
    }
    // Legacy diagnostic callers still need the entire original tick retained.
    return Number.isSafeInteger(current.oldestTick) && current.oldestTick<before.events[0].tick
      && JSON.stringify(current.events)===JSON.stringify(before.events);
  }));
}
export async function awaitDirectEffectBaseline(evidence, readEffects, {
  timeoutMs=10000, intervalMs=100, now=Date.now,
  wait=ms=>new Promise(resolve=>setTimeout(resolve,ms)),
}={}) {
  const deadline=now()+timeoutMs;
  do {
    evidence.effectBefore=await readEffects();
    evidence.effectAfter=evidence.effectBefore;
    evidence.effectSamples=[];
    if(directEffectOutcome(evidence))return true;
    if(now()>=deadline)return false;
    await wait(intervalMs);
  } while(now()<=deadline);
  return false;
}
// Install only after the requested reducer fact and journal order are present
// on both GMs. Bootstrap connection activity may have a later tick than that
// first fact; the continuous contract starts with the established baseline.
export async function startDirectEffectWitness(evidence, start, readEffects) {
  if (!directEffectOutcome({...evidence,effectWitnessRequired:false}))
    throw new Error('Missing actual effect baseline before continuous observation');
  evidence.effectBaseline=structuredClone(evidence.effectBefore);
  await start();
  const observed=await readEffects();
  if (!directEffectOutcome({...evidence,effectAfter:observed,effectWitnessRequired:false}))
    throw new Error('Effect baseline changed before continuous observation');
  evidence.effectBefore=observed;evidence.effectAfter=observed;evidence.effectSamples=[];
  evidence.effectWitnessRequired=true;
  if (!directEffectOutcome(evidence))
    throw new Error('Initial continuous witness does not contain the established effect');
}

export function divergenceOutcome(evidence) {
  const peers = evidence.after || [];
  const victim = peers.find(peer => peer.label === evidence.victim);
  const restore = victim?.mesh?.recovery?.divergence;
  const recovered = !!evidence.injected && restore?.result === 'recovered';
  const boundary = restore?.boundary_tick;
  const checkpoints = new Map();
  let invalidDigest = false;
  for (const peer of peers) for (const frame of peer.frames || []) {
    if (frame.t !== 'digest' || frame.d.from !== peer.mesh.slot || frame.d.tick <= boundary) continue;
    if (typeof frame.d.digest !== 'string' || !/^[0-9a-f]{16}$/.test(frame.d.digest)) { invalidDigest = true; continue; }
    if (!checkpoints.has(frame.d.tick)) checkpoints.set(frame.d.tick, new Map());
    const prior = checkpoints.get(frame.d.tick).get(peer.label);
    if (prior !== undefined && prior !== frame.d.digest) invalidDigest = true;
    checkpoints.get(frame.d.tick).set(peer.label, frame.d.digest);
  }
  const common = [...checkpoints].filter(([, rows]) => rows.size === 6)
    .map(([tick, rows]) => ({ tick, agreed: new Set(rows.values()).size === 1 }));
  const duplicateCommandOrders = [];
  for (const peer of peers) {
    const orders = new Set();
    for (const command of peer.commands || []) {
    const order = `${command.origin}:${command.seq}`;
    if (orders.has(order)) duplicateCommandOrders.push({peer:peer.label,order});
    orders.add(order);
    }
  }
  const shipIdentityStable = (evidence.identityBefore || []).length === 4
    && evidence.identityBefore.every(before => peers.find(peer => peer.label === before.label)?.commands
      .filter(command => command.type === 'SetThrust' && command.tick > boundary)
      .some(command => command.ship === before.ship));
  const effectAppliedOnce = directEffectOutcome(evidence);
  return { recovered, boundary, common, duplicateCommandOrders, shipIdentityStable, effectAppliedOnce,
    passed: peers.length === 6 && recovered && !invalidDigest && common.length >= 2 && common.every(row => row.agreed)
      && shipIdentityStable && effectAppliedOnce && duplicateCommandOrders.length === 0
      && peers.every(peer => peer.phase === 'InProgress' && peer.mesh.tick > boundary) };
}

export function divergenceHook({ faultSeconds = 90 } = {}) {
  return async ({ result, ships, gms, clients, step }) => {
    const peers = [...ships.map((page,index)=>({page,label:`ship-${index+1}`})), ...gms.map((page,index)=>({page,label:`gm-${index+1}`}))];
    const read = async peer => ({ label:peer.label, ...await peer.page.evaluate(()=>({
      mesh:window.__hostMeshStatus(),phase:window.__saveSlotsPhase,...window.__recoveryEvidence })) });
    const thrust = async value => {
      for (const client of clients.filter(row=>row.station==='helm')) await client.page.evaluate(value =>
        window.dispatchConsoleAction({action:'set_helm_thrust',value},(type,data)=>window.phoenixLink.send(type,data,'reliable')), value);
    };
    await Promise.all(peers.map(peer=>peer.page.evaluate(observeRecovery)));
    await thrust(.4);
    await Promise.all(ships.map(page=>page.waitForFunction(()=>window.__recoveryEvidence.commands.some(command=>command.type==='SetThrust'))));
    const before = await Promise.all(peers.map(read));
    const evidence = result.recovery = { failure:'divergence', victim:'gm-1', before,
      identityBefore:before.filter(peer=>peer.label.startsWith('ship-')).map(peer=>({label:peer.label,ship:peer.commands.find(command=>command.type==='SetThrust').ship})), samples:[] };
    evidence.effectRequest={entity:evidence.identityBefore[0].ship,correlation:'1534-divergence-effect-once',amount_milli_hp:5000};
    const readEffects=()=>Promise.all(gms.map(async(page,index)=>({label:`gm-${index+1}`,
      ...await page.evaluate(captureDirectEffect,evidence.effectRequest)})));
    try {
      const submitted=await gms[0].evaluate(request=>window.__hostApplyDirectEffect({
        ...request,effect:'damage',scope:'entity',scope_id:null}),evidence.effectRequest);
      if(!submitted)throw new Error('One-time GM direct-damage request was not submitted');
      await Promise.all(gms.map(page=>page.waitForFunction(correlation=>
        window.__hostGmJournalState?.().entries.some(row=>row.correlation===correlation && row.outcome==='applied'),
        evidence.effectRequest.correlation)));
      if(!await awaitDirectEffectBaseline(evidence,readEffects))
        throw new Error('Missing one actual GM damage event with retained earlier history within 10s');
      await startDirectEffectWitness(evidence,()=>Promise.all(gms.map(page=>
        page.evaluate(createEffectWitness,{...evidence.effectRequest,observe:true}))),readEffects);
      await gms[0].evaluate(() => {
        const receive = window.wasm_receive_mesh_frame;
        window.wasm_receive_mesh_frame = (source, raw) => {
          const frame = JSON.parse(raw);
          const command = frame.t === 'tick' && frame.d.from === 1
            && frame.d.commands.find(command=>command.payload?.type==='SetThrust');
          if (command && !window.__recoveryEvidence.injected) {
            const original = command.payload.data.value;
            command.payload.data.value = -.7;
            window.__recoveryEvidence.injected = { from:frame.d.from, tick:command.tick,
              seq:command.seq,ship:command.ship,target:command.target,original,changed:-.7 };
            return receive(source, JSON.stringify(frame));
          }
          return receive(source, raw);
        };
      });
      const firstHelm = clients.find(row=>row.ship===1 && row.station==='helm');
      await firstHelm.page.evaluate(()=>window.dispatchConsoleAction({action:'set_helm_thrust',value:.8},
        (type,data)=>window.phoenixLink.send(type,data,'reliable')));
      await gms[0].waitForFunction(()=>!!window.__recoveryEvidence.injected);
      step('recorded one GM damage effect, then changed one authenticated incoming SetThrust frame on gm-1');
      const deadline=Date.now()+faultSeconds*1000;
      let checkedIdentity=false;
      do {
        await new Promise(resolve=>setTimeout(resolve,500));
        evidence.after=await Promise.all(peers.map(read));
        evidence.injected=evidence.after.find(peer=>peer.label===evidence.victim).injected;
        evidence.effectAfter=await readEffects();evidence.effectSamples.push(evidence.effectAfter);
        evidence.samples.push(evidence.after.map(({label,mesh})=>({label,mesh})));
        evidence.outcome=divergenceOutcome(evidence);
        if(evidence.after.some(peer=>peer.overflow))throw new Error('Recovery observer overflow');
        if(evidence.outcome.recovered && !checkedIdentity){ checkedIdentity=true;await thrust(.5); }
        if(evidence.outcome.passed)return;
      } while(Date.now()<deadline);
      throw new Error('Divergence did not restore with stable ship identities, one continuously witnessed reducer effect, unique command orders and two matching checkpoints');
    } finally {
      evidence.effectObserverLogs=await Promise.all(gms.map(async(page,index)=>({label:`gm-${index+1}`,
        ...await page.evaluate(()=>window.__recoveryEffectWitness?.stop() || {notStarted:true})})));
      const invalid=evidence.effectObserverLogs.find(row=>row.error);
      if(invalid)throw new Error(`Continuous effect witness failed on ${invalid.label}: ${invalid.error}`);
    }
  };
}

export function recoveryMatrixArgs(args, failure) {
  // A pending fallback join is one page.evaluate call; its default 90s wrapper
  // must not expire before the replacement admission/challenge phase does.
  return failure === 'replacement' && !args.includes('--timeout')
    ? [...args, '--timeout', '180'] : [...args];
}

export async function main(args = process.argv.slice(2)) {
  const index = args.indexOf('--failure');
  if (index < 0 || !args[index+1]) throw new Error('Choose --failure ship|leader|gm|divergence|replacement');
  const copy = [...args]; const [,failure] = copy.splice(index,2);
  const afterHealthy = failure === 'replacement' ? replacementHook()
    : failure === 'divergence' ? divergenceHook() : failureHook(failure);
  return browserMatrix(recoveryMatrixArgs(copy, failure), {kind:'phoenix-real-browser-recovery-v1', provenance:{failure,
    faultSeconds:failure==='replacement'?null:failure==='divergence'?90:60,
    phaseBudgetsSeconds:failure==='replacement'?REPLACEMENT_PHASE_SECONDS:null,
    runnerSha256:createHash('sha256').update(readFileSync(fileURLToPath(import.meta.url))).digest('hex'),
    effectWitnessHelperSha256:failure==='divergence'?createHash('sha256').update(readFileSync(new URL('./fleet-effect-witness.mjs',import.meta.url))).digest('hex'):null,
    replacementHelperSha256:failure==='replacement'?createHash('sha256').update(readFileSync(new URL('./fleet-browser-replacement.mjs',import.meta.url))).digest('hex'):null}, afterHealthy});
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main().then(()=>process.exit(process.exitCode || 0),error=>{console.error(error);process.exit(1);});
