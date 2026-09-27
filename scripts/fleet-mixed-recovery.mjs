#!/usr/bin/env node
// Real mixed-runtime failure observations; native facts retain their actual source.
import path from 'node:path';
import { readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { main as mixedMatrix } from './fleet-mixed-matrix.mjs';
import { observeRecovery, captureDirectEffect, directEffectOutcome } from './fleet-browser-recovery.mjs';
import { createEffectWitness } from './fleet-effect-witness.mjs';

const nativeIds = ['ship-3','ship-4','gm-2'];
export const MIXED_FAILURES = ['ship','gm','leader','replacement','divergence'];
const integer = value => Number.isSafeInteger(value) && value > 0;
const last = (rows,kind) => rows.filter(row=>row.kind===kind).at(-1)?.value;
export function nativeRecoveryPeer(label, rows) {
  const roster = rows.filter(row=>row.kind==='simulation-roster' && rows.some(event=>event.kind==='state'
    && event.value.roster_result?.accepted && event.value.roster_result.generation===row.value.generation)).at(-1)?.value;
  const state = last(rows,'state');
  return {label,runtime:'native',slot:roster?.local,adopted:!!roster,
    recovery:state?.recovery,continuation:state?.continuation,
    frames:rows.filter(row=>row.kind==='digest').map(row=>({t:'digest',d:row.value})),
    controlFrames:rows.filter(row=>row.kind==='recovery-frame').map(row=>row.value),
    commands:rows.filter(row=>row.kind==='recovery-command').map(row=>row.value),
    routes:rows.filter(row=>row.kind==='onDiag'&&row.value.event==='transport').map(row=>row.value.transport),
    errors:rows.filter(row=>['fleet_fault','page-error','page-rejection','configure-error','station-error','gm-control-error','observer-error','observer-overflow'].includes(row.kind)),
    limits:['Native progress is measured by actual emitted digest checkpoints; no synthetic mesh tick or phase is supplied']};
}
export function browserRecoveryPeer(row) {
  return {...row,runtime:'browser',slot:row.mesh?.slot,adopted:row.mesh?.in_fleet===true,
    recovery:row.mesh?.recovery,controlFrames:row.frames?.filter(frame=>frame.t!=='digest')||[]};
}
function agreement(peers, afterTick) {
  const ticks=new Map();let malformed=false,contradictory=false;
  for(const peer of peers)for(const frame of peer.frames||[]) {
    if(frame.t!=='digest')continue;
    const d=frame.d;
    if(d?.from!==peer.slot||!integer(d.tick)||typeof d.digest!=='string'||!/^[0-9a-f]{16}$/.test(d.digest)){malformed=true;continue;}
    if(d.tick<=afterTick)continue;
    if(!ticks.has(d.tick))ticks.set(d.tick,new Map());
    const checkpoint=ticks.get(d.tick);
    if(checkpoint.has(peer.label)&&checkpoint.get(peer.label)!==d.digest)contradictory=true;
    checkpoint.set(peer.label,d.digest);
  }
  const common=[...ticks].filter(([,values])=>values.size===peers.length).map(([tick,values])=>({tick,byPeer:Object.fromEntries(values),agreed:new Set(values.values()).size===1}));
  return {common,malformed,contradictory,passed:peers.length>0&&!malformed&&!contradictory&&common.length>=2&&common.every(row=>row.agreed)};
}
export function mixedRecoveryOutcome(evidence) {
  const before=evidence.before||[],after=evidence.after||[],victim=evidence.victim;
  const expected=before.filter(peer=>peer.label!==victim?.label);
  const reasons=[];const need=(ok,reason)=>{if(!ok)reasons.push(reason);};
  need(before.length===6&&new Set(before.map(peer=>peer.label)).size===6&&new Set(before.map(peer=>peer.slot)).size===6
    &&before.every(peer=>integer(peer.slot)&&peer.adopted),'six distinct admitted baseline peers required');
  need(after.length===5&&new Set(after.map(peer=>peer.label)).size===5&&expected.every(peer=>after.some(row=>row.label===peer.label&&row.slot===peer.slot)),'survivor identities changed');
  need(evidence.stopped?.confirmed===true,'victim process/page exit unverified');
  const applied=after.map(peer=>peer.recovery?.losses?.filter(row=>row.slot===victim?.slot)||[]);
  const lossTick=applied[0]?.[0]?.tick;
  need(applied.length===5&&applied.every(rows=>rows.length===1&&rows[0].tick===lossTick)&&integer(lossTick),'one identical applied HostLoss per survivor required');
  const ship=before.some(peer=>peer.recovery?.ships?.some(row=>row.slot===victim?.slot));
  need(!ship||after.every(peer=>peer.recovery?.ships?.some(row=>row.slot===victim.slot&&row.crewed===false)),'lost ship Backfill unobserved');
  need(after.every(peer=>peer.runtime!=='browser'||(peer.phase==='InProgress'&&peer.mesh?.tick>Math.max(evidence.atTick,lossTick||0)+30
    &&peer.mesh.peers.length===4&&!peer.mesh.peers.includes(victim.slot))),'browser survivors did not resume with the five-peer wait set');
  if(evidence.failure==='leader')need(after.every(peer=>peer.continuation?.status==='committed'&&peer.continuation.loss_tick===lossTick),'owner continuation commit missing');
  const checkpoints=agreement(after,Math.max(evidence.atTick,lossTick||0));
  need(checkpoints.passed,'two exact matching post-loss digest checkpoints required');
  const duplicateOrders=[];
  for(const peer of after){const seen=new Set();for(const command of peer.commands||[]){const key=JSON.stringify([command.origin,command.seq]);if(seen.has(key))duplicateOrders.push({peer:peer.label,key});seen.add(key);}}
  need(!duplicateOrders.length,'duplicate outgoing command orders');
  need(after.every(peer=>!peer.overflow&&!peer.errors?.length),'runtime/observer failure');
  return {passed:!reasons.length,reasons,lossTick,backfillApplicable:ship,commonDigests:checkpoints.common,duplicateOrders,
    limits:['Command uniqueness observes production egress orders; it does not independently count every simulation side effect']};
}
export function mixedReplacementOutcome(evidence) {
  const after=evidence.afterReplacement||[],replacement=after.find(peer=>peer.label===(evidence.winner||'ship-replacement'));
  const restore=replacement?.recovery?.replacement,boundary=restore?.boundary_tick;
  const survivorLabels=evidence.before?.filter(peer=>peer.label!==evidence.victim.label).map(peer=>peer.label)||[];
  const reasons=[];const need=(ok,reason)=>{if(!ok)reasons.push(reason);};
  need(evidence.outcome?.passed===true,'prior loss acceptance missing');
  need(after.length===6&&new Set(after.map(peer=>peer.label)).size===6&&replacement?.adopted&&replacement.slot===evidence.victim.slot
    &&survivorLabels.every(label=>after.some(peer=>peer.label===label&&peer.slot===evidence.before.find(row=>row.label===label).slot)),'replacement changed a technical slot');
  need(restore?.result==='recovered'&&integer(boundary)&&boundary>evidence.outcome?.lossTick,'replacement restore boundary missing');
  need(after.every(peer=>{const r=peer.recovery?.replacement;return r?.slot===evidence.victim.slot&&r.boundary_tick===boundary&&r.claim_seq===restore?.claim_seq&&r.leader===restore?.leader
    &&r.result===(peer===replacement?'recovered':peer.slot===restore?.leader?'led':'witnessed');}),'replacement boundary not agreed');
  const claims=after.flatMap(peer=>(peer.controlFrames||[]).filter(frame=>frame.t==='slot-claim').map(frame=>({observer:peer.slot,...frame.d})));
  need(claims.length===1&&claims[0].slot===evidence.victim.slot&&claims[0].from===claims[0].observer&&claims[0].claim_seq===restore?.claim_seq,'exactly one authoritative slot claim required');
  const checkpoints=agreement(after,Math.max(boundary??Infinity,evidence.challengeTick||0));need(checkpoints.passed,'replacement digest agreement missing');
  const oldShips=new Set(evidence.before.find(peer=>peer.label===evidence.victim.label)?.commands?.map(row=>row.ship));
  need(oldShips.size===1&&replacement?.commands?.some(row=>row.tick>boundary&&oldShips.has(row.ship)),'restored ship entity identity unobserved');
  const duplicateOrders=[];
  for(const peer of after){
    const seen=new Set(),commands=peer===replacement?[...(evidence.before.find(row=>row.label===evidence.victim.label)?.commands||[]),...(peer.commands||[])]:peer.commands||[];
    for(const command of commands){const key=JSON.stringify([command.origin,command.seq]);if(seen.has(key))duplicateOrders.push({peer:peer.label,key});seen.add(key);}
  }
  need(!duplicateOrders.length,'replacement repeated an outgoing command order');
  need(after.every(peer=>!peer.overflow&&!peer.errors?.length),'replacement runtime/observer failure');
  if(evidence.raceRequired) {
    const race=evidence.race||[],winners=race.filter(row=>row.admitted),losers=race.filter(row=>row.refused&&!row.admitted);
    need(race.length===2&&winners.length===1&&losers.length===1&&winners[0].label===replacement?.label
      &&race.every(row=>row.attempt?.accepted===true&&Number.isSafeInteger(row.attempt.startedMs))
      &&Math.abs(race[0].attempt.startedMs-race[1].attempt.startedMs)<=250,'two simultaneous native claim attempts with exactly one winner required');
    need(evidence.challenger?.label===losers[0]?.label&&evidence.challenger?.refused===true
      &&evidence.challenger?.attempt?.accepted===true&&integer(evidence.challengeTick)
      &&evidence.challenger.attempt.startedMs>Math.max(...race.map(row=>row.attempt?.startedMs||0)),
      'connected holder challenge was not refused');
    need(replacement?.routes?.includes('ws-relay'),'native replacement relay route unobserved');
  }
  return {passed:!reasons.length,reasons,boundary,duplicateOrders,commonDigests:checkpoints.common,
    limits:evidence.raceRequired?[]:['One native replacement only; no simultaneous claim race evidence']};
}
export function mixedFailureHook(failure,{faultSeconds=90,nativeLabels=nativeIds}={}) {
  if(!MIXED_FAILURES.includes(failure)||!integer(faultSeconds)||faultSeconds>600)throw new Error('Invalid mixed recovery scenario');
  if(failure==='divergence')return mixedDivergenceHook({faultSeconds,nativeLabels});
  return async ({result,peers,step,stopNative,launchNative,commandNative,wait,deadline,evaluate})=>{
    await Promise.all(peers.map(peer=>evaluate(peer.page,observeRecovery)));
    const readBrowser=async peer=>browserRecoveryPeer({label:peer.label,errors:peer.errors,...await evaluate(peer.page,()=>({mesh:window.__hostMeshStatus(),phase:window.__saveSlotsPhase,
      continuation:window.wasm_fleet_continuation_status?JSON.parse(window.wasm_fleet_continuation_status()):null,...window.__recoveryEvidence}))});
    const capture=async(browserPeers,natives)=>[...await Promise.all(browserPeers.map(readBrowser)),...natives.map(id=>nativeRecoveryPeer(id,result.nativeEvents[id]||[]))];
    const before=await capture(peers,nativeLabels),label={ship:'ship-3',replacement:'ship-3',gm:'gm-2',leader:'ship-1'}[failure];
    const victim=before.find(peer=>peer.label===label);
    const ticks=before.flatMap(peer=>peer.runtime==='browser'?[peer.mesh.tick]:peer.frames.filter(frame=>frame.t==='digest').map(frame=>frame.d.tick));
    const evidence=result.recovery={failure,before,victim:{label,slot:victim.slot},atTick:Math.max(...ticks),startedUtc:new Date().toISOString(),samples:[]};
    const candidates=['ship-replacement-1','ship-replacement-2'];
    if(failure==='replacement') {
      evidence.raceRequired=true;
      for(const id of candidates)launchNative(id,{claim:'slot-'+victim.slot,deferJoin:true});
      await wait(()=>candidates.every(id=>result.nativeEvents[id]?.some(row=>row.kind==='join-deferred')),'native replacement candidates ready');
    }
    if(nativeLabels.includes(label))evidence.stopped={confirmed:true,...await stopNative(label)};
    else {const peer=peers.find(peer=>peer.label===label);await peer.page.close();evidence.stopped={confirmed:peer.page.isClosed()};}
    step('stopped '+label+' at slot '+victim.slot);
    const browserSurvivors=peers.filter(peer=>peer.label!==label),nativeSurvivors=nativeLabels.filter(id=>id!==label);
    const faultDeadline=Math.min(deadline,Date.now()+faultSeconds*1000);
    await wait(async()=>{
      evidence.after=await capture(browserSurvivors,nativeSurvivors);evidence.outcome=mixedRecoveryOutcome(evidence);
      if(evidence.samples.length>=4000)throw new Error('Mixed recovery sample overflow');
      evidence.samples.push({at:new Date().toISOString(),outcome:evidence.outcome});
      if(Date.now()>faultDeadline)throw new Error('Mixed loss deadline: '+evidence.outcome.reasons.join('; '));
      return evidence.outcome.passed;
    },'five survivors apply loss and agree');
    step('five mixed survivors applied loss/Backfill and agreed');
    if(failure!=='replacement')return;
    const startAt=Date.now()+1000;
    for(const id of candidates)commandNative(id,{kind:'join',startAt,attempt:'race'});
    const candidate=(id,attempt)=>{
      const rows=result.nativeEvents[id]||[],call=rows.filter(row=>row.kind==='join-call'&&row.value.attempt===attempt).at(-1);
      const following=call?rows.slice(rows.indexOf(call)+1):[];
      return {label:id,attempt:call?.value,admitted:following.some(row=>row.kind==='fleet_join_status'&&row.value.status==='admitted'),
        refused:following.some(row=>row.kind==='fleet_fault'&&row.value.reason==='slot-taken')};
    };
    await wait(()=>{evidence.race=candidates.map(id=>candidate(id,'race'));return evidence.race.every(row=>row.admitted||row.refused);},'native race admission/refusal');
    if(evidence.race.filter(row=>row.admitted).length!==1)throw new Error('Native race did not produce exactly one winner');
    evidence.winner=evidence.race.find(row=>row.admitted).label;
    const loser=candidates.find(id=>id!==evidence.winner);
    await wait(async()=>{
      evidence.afterReplacement=await capture(browserSurvivors,[...nativeSurvivors,evidence.winner]);
      const replacement=evidence.afterReplacement.find(row=>row.label===evidence.winner);
      if(!evidence.challengeTick&&replacement?.recovery?.replacement?.result==='recovered') {
        evidence.challengeTick=Math.max(...evidence.afterReplacement.flatMap(peer=>peer.frames.filter(frame=>frame.t==='digest').map(frame=>frame.d.tick)));
        commandNative(loser,{kind:'join',startAt:Date.now()+500,attempt:'challenge'});
      }
      evidence.challenger=candidate(loser,'challenge');evidence.replacementOutcome=mixedReplacementOutcome(evidence);
      if(Date.now()>faultDeadline)throw new Error('Native replacement deadline: '+evidence.replacementOutcome.reasons.join('; '));
      return evidence.replacementOutcome.passed;
    },'native replacement restores same slot and ship');
    step('native replacement restored with two common mixed digest checkpoints');
  };
}
export function mixedDivergenceOutcome(evidence) {
  const before=evidence.before||[],after=evidence.after||[],reasons=[];
  const need=(ok,reason)=>{if(!ok)reasons.push(reason);};
  const victim=after.find(peer=>peer.label===evidence.victim),restore=victim?.recovery?.divergence,boundary=restore?.boundary_tick;
  need(before.length===6&&after.length===6&&new Set(after.map(peer=>peer.slot)).size===6
    &&before.every(peer=>peer.adopted&&after.some(row=>row.label===peer.label&&row.slot===peer.slot&&row.adopted)),'six unchanged admitted peer identities required');
  need(evidence.injected?.authenticatedSlot===evidence.injected?.from&&integer(evidence.injected?.tick)
    &&restore?.result==='recovered'&&integer(boundary)&&boundary>evidence.injected.tick,'native GM divergence restore missing');
  need(after.every(peer=>{const r=peer.recovery?.divergence;return r?.boundary_tick===boundary&&r.leader===restore?.leader
    &&r.result===(peer===victim?'recovered':peer.slot===restore?.leader?'led':'witnessed');}),'divergence boundary not agreed');
  const exact=agreement(after,boundary??Infinity);need(exact.passed,'two exact matching post-restore checkpoints required');
  const identities=before.filter(peer=>peer.label.startsWith('ship-')).map(peer=>({label:peer.label,ships:[...new Set((peer.commands||[]).map(row=>row.ship))]}));
  need(identities.length===4&&identities.every(identity=>identity.ships.length===1&&after.find(peer=>peer.label===identity.label)?.commands?.some(row=>row.tick>boundary&&row.ship===identity.ships[0])),'post-restore original ship controls missing');
  const duplicateOrders=[];
  for(const peer of after){const seen=new Set();for(const row of peer.commands||[]){const key=JSON.stringify([row.origin,row.seq]);if(seen.has(key))duplicateOrders.push({peer:peer.label,key});seen.add(key);}}
  need(!duplicateOrders.length,'duplicate outgoing command orders');
  need(directEffectOutcome(evidence),'one continuously witnessed actual effect required');
  need(after.every(peer=>!peer.errors?.length&&!peer.overflow),'runtime/observer failure');
  return {passed:!reasons.length,reasons,boundary,commonDigests:exact.common,duplicateOrders};
}
export function mixedDivergenceHook({faultSeconds=600,nativeLabels=nativeIds}={}) {
  return async({result,peers,clients=[],step,wait,deadline,evaluate,commandNative,commandGm})=>{
    await Promise.all(peers.map(peer=>evaluate(peer.page,observeRecovery)));
    const browserGms=peers.filter(peer=>peer.label.startsWith('gm-')),nativeGms=nativeLabels.filter(id=>id.startsWith('gm-'));
    const capture=async()=>[...await Promise.all(peers.map(async peer=>browserRecoveryPeer({label:peer.label,errors:peer.errors,
      ...await evaluate(peer.page,()=>({mesh:window.__hostMeshStatus(),phase:window.__saveSlotsPhase,...window.__recoveryEvidence}))}))),
      ...nativeLabels.map(id=>nativeRecoveryPeer(id,result.nativeEvents[id]||[]))];
    const thrust=async value=>{for(const client of clients.filter(row=>row.station==='helm'))await evaluate(client.page,value=>
      window.dispatchConsoleAction({action:'set_helm_thrust',value},(type,data)=>window.phoenixLink.send(type,data,'reliable')),value);};
    await thrust(.4);
    let before;
    await wait(async()=>{before=await capture();return before.filter(peer=>peer.label.startsWith('ship-')).every(peer=>peer.commands?.length);},'ship identities before divergence');
    const evidence=result.recovery={failure:'divergence',victim:'gm-2',before,effectWitnessRequired:true,effectSamples:[],samples:[]};
    evidence.effectRequest={entity:before.find(peer=>peer.label==='ship-1').commands[0].ship,correlation:'1534-native-divergence-effect-once',amount_milli_hp:5000};
    for(const id of nativeGms)commandGm(id,{kind:'effect-observe',request:evidence.effectRequest});
    for(const peer of browserGms)await evaluate(peer.page,createEffectWitness,{...evidence.effectRequest,observe:true,maxDurationMs:600000,maxSamples:14000,maxBytes:32*1024*1024});
    await wait(()=>nativeGms.every(id=>result.nativeEvents[id].some(row=>row.kind==='effect-observer-started')),'native effect observers active');
    commandGm(nativeGms[0],{kind:'effect-apply',request:evidence.effectRequest});
    const effects=async()=>{
      const rows=[...await Promise.all(browserGms.map(async peer=>({label:peer.label,...await evaluate(peer.page,captureDirectEffect,evidence.effectRequest)}))),
        ...nativeGms.map(id=>({label:id,events:[],journal:[],...last(result.nativeEvents[id],'effect-witness')}))];
      const invalid=rows.find(row=>row.continuous?.error);
      if(invalid)throw new Error(invalid.label+' continuous effect witness failed: '+invalid.continuous.error);
      return rows;
    };
    const faultDeadline=Math.min(deadline,Date.now()+faultSeconds*1000);
    const bounded=async fn=>{if(Date.now()>faultDeadline)throw new Error('Native divergence deadline: '+(evidence.outcome?.reasons?.join('; ')||'effect baseline'));return fn();};
    await wait(()=>bounded(async()=>{evidence.effectBefore=await effects();evidence.effectAfter=evidence.effectBefore;return directEffectOutcome(evidence);}), 'one actual GM damage baseline');
    commandNative(evidence.victim,{kind:'diverge'});step('armed one incoming authenticated native GM command change');
    let checkedIdentity=false;
    await wait(()=>bounded(async()=>{
      evidence.after=await capture();evidence.injected=last(result.nativeEvents[evidence.victim],'divergence-injected');
      evidence.effectAfter=await effects();evidence.effectSamples.push(evidence.effectAfter);
      if(evidence.effectSamples.length>4000)throw new Error('Native divergence sample bound');
      evidence.outcome=mixedDivergenceOutcome(evidence);
      const boundary=evidence.after.find(peer=>peer.label===evidence.victim)?.recovery?.divergence?.boundary_tick;
      if(boundary&&!checkedIdentity){checkedIdentity=true;await thrust(.5);}
      return evidence.outcome.passed;
    }),'native divergence recovery, effect uniqueness and checkpoints');
    step('native GM restored with one effect and two common checkpoints');
  };
}
export function mixedRecoveryOptions(args) {
  const copy=[...args];
  const extract=(name,required=false)=>{
    const indexes=copy.flatMap((value,index)=>value===name?[index]:[]);
    if(indexes.length>1)throw new Error('Repeated '+name);
    if(!indexes.length){if(required)throw new Error('Choose --failure ship|gm|leader|replacement');return null;}
    const index=indexes[0],value=copy[index+1];
    if(!value||value.startsWith('--'))throw new Error('Incomplete '+name);
    copy.splice(index,2);return value;
  };
  const failure=extract('--failure',true);
  if(!MIXED_FAILURES.includes(failure))throw new Error('Invalid mixed recovery scenario');
  const faultSeconds=Number(extract('--fault-seconds')??90);
  if(!integer(faultSeconds)||faultSeconds>600)throw new Error('--fault-seconds must be an integer from 1 to 600');
  return {args:copy,failure,faultSeconds};
}
export async function main(args=process.argv.slice(2)) {
  const {args:copy,failure,faultSeconds}=mixedRecoveryOptions(args);
  for(const option of ['--build-receipt','--wasm-build-receipt'])if(!copy.includes(option))throw new Error(option+' is required for recovery evidence');
  if(!copy.includes('--render'))throw new Error('--render is required for recovery evidence');
  return mixedMatrix(copy,{kind:'phoenix-real-mixed-recovery-v1',provenance:{failure,faultSeconds,runnerSha256:createHash('sha256').update(readFileSync(fileURLToPath(import.meta.url))).digest('hex')},afterHealthy:mixedFailureHook(failure,{faultSeconds})});
}
if(process.argv[1]&&path.resolve(process.argv[1])===fileURLToPath(import.meta.url))main().then(result=>process.exit(result.status==='passed'?0:1),error=>{console.error(error);process.exit(1);});
