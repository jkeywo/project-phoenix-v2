#!/usr/bin/env node
// Real mixed-runtime failure observations; native facts retain their actual source.
import path from 'node:path';
import { readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { main as mixedMatrix } from './fleet-mixed-matrix.mjs';
import { observeRecovery } from './fleet-browser-recovery.mjs';

const nativeIds = ['ship-3','ship-4','gm-2'];
export const MIXED_FAILURES = ['ship','gm','leader','replacement'];
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
  const after=evidence.afterReplacement||[],replacement=after.find(peer=>peer.label==='ship-replacement');
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
  const checkpoints=agreement(after,boundary??Infinity);need(checkpoints.passed,'replacement digest agreement missing');
  const oldShips=new Set(evidence.before.find(peer=>peer.label===evidence.victim.label)?.commands?.map(row=>row.ship));
  need(oldShips.size===1&&replacement?.commands?.some(row=>row.tick>boundary&&oldShips.has(row.ship)),'restored ship entity identity unobserved');
  const duplicateOrders=[];
  for(const peer of after){
    const seen=new Set(),commands=peer===replacement?[...(evidence.before.find(row=>row.label===evidence.victim.label)?.commands||[]),...(peer.commands||[])]:peer.commands||[];
    for(const command of commands){const key=JSON.stringify([command.origin,command.seq]);if(seen.has(key))duplicateOrders.push({peer:peer.label,key});seen.add(key);}
  }
  need(!duplicateOrders.length,'replacement repeated an outgoing command order');
  need(after.every(peer=>!peer.overflow&&!peer.errors?.length),'replacement runtime/observer failure');
  return {passed:!reasons.length,reasons,boundary,duplicateOrders,commonDigests:checkpoints.common,
    limits:['One native replacement after confirmed loss; simultaneous claim race and connected-holder challenge are separate browser cases']};
}
export function mixedFailureHook(failure,{faultSeconds=90}={}) {
  if(!MIXED_FAILURES.includes(failure)||!integer(faultSeconds)||faultSeconds>600)throw new Error('Invalid mixed recovery scenario');
  return async ({result,peers,step,stopNative,launchNative,wait,deadline,evaluate})=>{
    await Promise.all(peers.map(peer=>evaluate(peer.page,observeRecovery)));
    const readBrowser=async peer=>browserRecoveryPeer({label:peer.label,errors:peer.errors,...await evaluate(peer.page,()=>({mesh:window.__hostMeshStatus(),phase:window.__saveSlotsPhase,
      continuation:window.wasm_fleet_continuation_status?JSON.parse(window.wasm_fleet_continuation_status()):null,...window.__recoveryEvidence}))});
    const capture=async(browserPeers,natives)=>[...await Promise.all(browserPeers.map(readBrowser)),...natives.map(id=>nativeRecoveryPeer(id,result.nativeEvents[id]||[]))];
    const before=await capture(peers,nativeIds),label={ship:'ship-3',replacement:'ship-3',gm:'gm-2',leader:'ship-1'}[failure];
    const victim=before.find(peer=>peer.label===label);
    const ticks=before.flatMap(peer=>peer.runtime==='browser'?[peer.mesh.tick]:peer.frames.filter(frame=>frame.t==='digest').map(frame=>frame.d.tick));
    const evidence=result.recovery={failure,before,victim:{label,slot:victim.slot},atTick:Math.max(...ticks),startedUtc:new Date().toISOString(),samples:[]};
    if(nativeIds.includes(label))evidence.stopped={confirmed:true,...await stopNative(label)};
    else {const peer=peers.find(peer=>peer.label===label);await peer.page.close();evidence.stopped={confirmed:peer.page.isClosed()};}
    step('stopped '+label+' at slot '+victim.slot);
    const browserSurvivors=peers.filter(peer=>peer.label!==label),nativeSurvivors=nativeIds.filter(id=>id!==label);
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
    launchNative('ship-replacement',{claim:'slot-'+victim.slot});
    await wait(async()=>{
      evidence.afterReplacement=await capture(browserSurvivors,[...nativeSurvivors,'ship-replacement']);
      evidence.replacementOutcome=mixedReplacementOutcome(evidence);
      if(Date.now()>faultDeadline)throw new Error('Native replacement deadline: '+evidence.replacementOutcome.reasons.join('; '));
      return evidence.replacementOutcome.passed;
    },'native replacement restores same slot and ship');
    step('native replacement restored with two common mixed digest checkpoints');
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
