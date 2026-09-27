#!/usr/bin/env node
// #1534 actual browser faults after the #1530 healthy six-peer gate.
import path from 'node:path';
import { readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { main as browserMatrix } from './fleet-browser-matrix.mjs';

export const FAILURES = ['ship', 'leader', 'gm'];
export function failureOutcome(evidence) {
  const survivors = evidence.after || [];
  const expected = evidence.before?.filter(peer => peer.label !== evidence.victim.label) || [];
  const ticks = new Map();
  for (const peer of survivors) for (const digest of peer.frames?.filter(frame => frame.t === 'digest') || []) {
    if (digest.d.from !== peer.mesh?.slot || digest.d.tick <= evidence.atTick) continue;
    if (!ticks.has(digest.d.tick)) ticks.set(digest.d.tick, new Map());
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
  return { resumed, commonDigests, survivorAgreement: resumed && commonDigests.length >= 2 && commonDigests.every(row => row.agreed),
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
export function divergenceOutcome(evidence) {
  const peers = evidence.after || [];
  const victim = peers.find(peer => peer.label === evidence.victim);
  const restore = victim?.mesh?.recovery?.divergence;
  const recovered = !!evidence.injected && restore?.result === 'recovered';
  const boundary = restore?.boundary_tick;
  const checkpoints = new Map();
  for (const peer of peers) for (const frame of peer.frames || []) {
    if (frame.t !== 'digest' || frame.d.from !== peer.mesh.slot || frame.d.tick <= boundary) continue;
    if (!checkpoints.has(frame.d.tick)) checkpoints.set(frame.d.tick, new Map());
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
  return { recovered, boundary, common, duplicateCommandOrders, shipIdentityStable,
    passed: peers.length === 6 && recovered && common.length >= 2 && common.every(row => row.agreed)
      && shipIdentityStable && duplicateCommandOrders.length === 0
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
    const evidence = result.recovery = { failure:'divergence', victim:'ship-2', before,
      identityBefore:before.filter(peer=>peer.label.startsWith('ship-')).map(peer=>({label:peer.label,ship:peer.commands.find(command=>command.type==='SetThrust').ship})), samples:[] };
    await ships[1].evaluate(() => {
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
    await ships[1].waitForFunction(()=>!!window.__recoveryEvidence.injected);
    step('changed one authenticated incoming SetThrust frame on ship-2');
    const deadline=Date.now()+faultSeconds*1000;
    let checkedIdentity=false;
    do {
      await new Promise(resolve=>setTimeout(resolve,500));
      evidence.after=await Promise.all(peers.map(read));
      evidence.injected=evidence.after.find(peer=>peer.label==='ship-2').injected;
      evidence.samples.push(evidence.after.map(({label,mesh})=>({label,mesh})));
      evidence.outcome=divergenceOutcome(evidence);
      if(evidence.after.some(peer=>peer.overflow))throw new Error('Recovery observer overflow');
      if(evidence.outcome.recovered && !checkedIdentity){ checkedIdentity=true;await thrust(.5); }
      if(evidence.outcome.passed)return;
    } while(Date.now()<deadline);
    throw new Error('Divergence did not restore with stable ship identities, unique command orders and two matching checkpoints');
  };
}

export async function main(args = process.argv.slice(2)) {
  const index = args.indexOf('--failure');
  if (index < 0 || !args[index+1]) throw new Error('Choose --failure ship|leader|gm|divergence');
  const copy = [...args]; const [,failure] = copy.splice(index,2);
  return browserMatrix(copy, {kind:'phoenix-real-browser-recovery-v1', provenance:{failure, faultSeconds:failure==='divergence'?90:60, runnerSha256:createHash('sha256').update(readFileSync(fileURLToPath(import.meta.url))).digest('hex')}, afterHealthy:failure==='divergence'?divergenceHook():failureHook(failure)});
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main().then(()=>process.exit(process.exitCode || 0),error=>{console.error(error);process.exit(1);});
