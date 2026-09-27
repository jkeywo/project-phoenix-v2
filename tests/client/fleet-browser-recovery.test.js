import { optionsFrom } from '../../scripts/fleet-browser-matrix.mjs';
import { createEffectWitness } from '../../scripts/fleet-effect-witness.mjs';
import { describe, expect, it } from 'vitest';
import { recoveryMatrixArgs, failureOutcome, divergenceOutcome, observeRecovery, directEffectOutcome, awaitDirectEffectBaseline, startDirectEffectWitness } from '../../scripts/fleet-browser-recovery.mjs';

function evidence() {
  const before = Array.from({length:6}, (_,i)=>({label:`peer-${i+1}`,mesh:{slot:i+1,tick:100}}));
  const after = before.slice(1).map(peer=>({...peer,phase:'InProgress',mesh:{...peer.mesh,tick:200,peers:before.slice(1).filter(row=>row!==peer).map(row=>row.mesh.slot)},frames:[150,180].map(tick=>({t:'digest',d:{from:peer.mesh.slot,tick,digest:'0123456789abcdef'}}))}));
  return {before,after,victim:{label:'peer-1',slot:1},atTick:100};
}
describe('real runtime recovery evidence gate',()=>{
  it('requires two fresh common digest checkpoints after the departed slot leaves every wait-set',()=>{
    expect(failureOutcome(evidence()).survivorAgreement).toBe(true);
    const missing=evidence(); missing.after[4].frames.pop();
    expect(failureOutcome(missing).survivorAgreement).toBe(false);
    const waiting=evidence(); waiting.after[0].mesh.peers.push(1);
    expect(failureOutcome(waiting).survivorAgreement).toBe(false);
    const stale=evidence(); stale.after.forEach(peer=>peer.frames.forEach(frame=>frame.d.tick=90));
    expect(failureOutcome(stale).survivorAgreement).toBe(false);
  });
  it('retains the first post-loss divergence instead of accepting a later matching checkpoint',()=>{
    const broken=evidence(); broken.after[3].frames[0].d.digest='fedcba9876543210';
    const outcome=failureOutcome(broken);
    expect(outcome.survivorAgreement).toBe(false);
    expect(outcome.commonDigests.find(row=>!row.agreed).tick).toBe(150);
  });
  it('does not call digest agreement proof of applied Backfill',()=>{
    expect(failureOutcome(evidence()).backfillVerified).toBe(false);
  });
});


it('requires every authoritative applied loss boundary and the ship Backfill transition',()=>{
  const proof=evidence();
  proof.before.forEach(peer=>peer.mesh.recovery={ships:[{slot:1,crewed:true}]});
  proof.after.forEach(peer=>peer.mesh.recovery={losses:[{slot:1,tick:103}],ships:[{slot:1,crewed:false}]});
  expect(failureOutcome(proof)).toMatchObject({lossApplied:true,lossTick:103,backfillVerified:true});
  proof.after[3].mesh.recovery.ships[0].crewed=true;
  expect(failureOutcome(proof).backfillVerified).toBe(false);
  proof.after[4].mesh.recovery.losses[0].tick=104;
  expect(failureOutcome(proof).lossApplied).toBe(false);
});

it('counts duplicate command orders within each peer, allowing ordinary replicated orders',()=>{
  const command={origin:1,seq:7};
  const peers=[{label:'one',mesh:{slot:1},commands:[command]},{label:'two',mesh:{slot:2},commands:[command]}];
  expect(divergenceOutcome({after:peers}).duplicateCommandOrders).toEqual([]);
  peers[1].commands.push(command);
  expect(divergenceOutcome({after:peers}).duplicateCommandOrders).toEqual([{peer:'two',order:'1:7'}]);
});

it('preserves production hex digest strings without modifying production egress',()=>{
  const raw='[{"t":"digest","d":{"from":1,"tick":300,"digest":"fffffffffffffffe"}}]';
  const saved=globalThis.window;
  try {globalThis.window={wasm_take_mesh_frames:()=>raw};observeRecovery();
    expect(window.wasm_take_mesh_frames()).toBe(raw);
    expect(window.__recoveryEvidence.frames[0].d.digest).toBe('fffffffffffffffe');
  } finally {if(saved===undefined)delete globalThis.window;else globalThis.window=saved;}
});

it('rejects missing digest values and contradictory repeated checkpoints',()=>{
  const missing=evidence();missing.after.forEach(peer=>peer.frames.forEach(frame=>delete frame.d.digest));
  expect(failureOutcome(missing).survivorAgreement).toBe(false);
  const contradictory=evidence();
  contradictory.after[0].frames.unshift({t:'digest',d:{...contradictory.after[0].frames[0].d,digest:'ffffffffffffffff'}});
  expect(failureOutcome(contradictory).survivorAgreement).toBe(false);
});

function directEffectEvidence(){
  const effectRequest={entity:'stable-ship',correlation:'one-effect',amount_milli_hp:5000};
  const effectBefore=['gm-1','gm-2'].map(label=>({label,capacity:256,oldestTick:2,
    events:[{tick:400,category:'damage',detail:{type:'damage',data:{weapon:'gm.direct',amount:5,hull_damage:5}},links:[{role:'victim',entity:{entity_id:'stable-ship'}}]}],
    journal:[{correlation:'one-effect',action_kind:'direct-effect',tick:400,sequence:3,outcome:'applied'}]}));
  return {effectRequest,effectBefore,effectAfter:structuredClone(effectBefore),effectSamples:[]};
}
it('requires exactly one actual reducer damage event retained across the restore',()=>{
  const proof=directEffectEvidence();expect(directEffectOutcome(proof)).toBe(true);
  proof.effectAfter[0].events.push(structuredClone(proof.effectAfter[0].events[0]));
  expect(directEffectOutcome(proof)).toBe(false);
  const journalOnly=directEffectEvidence();journalOnly.effectAfter[0].events=[];
  expect(directEffectOutcome(journalOnly)).toBe(false);
});
it('rejects evidence whose bounded feed could have evicted a duplicate or hid an earlier duplicate',()=>{
  const proof=directEffectEvidence();proof.effectAfter[0].oldestTick=400;
  expect(directEffectOutcome(proof)).toBe(false);
  const hidden=directEffectEvidence(),bad=structuredClone(hidden.effectBefore);
  bad[1].events.push(structuredClone(bad[1].events[0]));hidden.effectSamples.push(bad);
  expect(directEffectOutcome(hidden)).toBe(false);
});

it('ties the actual damage event to the requested entity and one shared action order',()=>{
  const wrong=directEffectEvidence();wrong.effectRequest.entity='another-ship';
  expect(directEffectOutcome(wrong)).toBe(false);
  const split=directEffectEvidence();split.effectBefore[1].journal[0].sequence=4;
  expect(directEffectOutcome(split)).toBe(false);
});

it('waits boundedly for both activity projections after the journal reports applied',async()=>{
  const proof=directEffectEvidence(),complete=structuredClone(proof.effectBefore);
  let turn=0;
  const read=async()=>complete.map((row,index)=>({...row,events:turn>index?row.events:[]}));
  expect(await awaitDirectEffectBaseline(proof,read,{now:()=>turn*100,wait:async()=>{turn++;},timeoutMs:500})).toBe(true);
  expect(turn).toBe(2);expect(directEffectOutcome(proof)).toBe(true);
  const missing=directEffectEvidence();turn=0;
  expect(await awaitDirectEffectBaseline(missing,async()=>complete.map(row=>({...row,events:[]})),
    {now:()=>turn*100,wait:async()=>{turn++;},timeoutMs:200})).toBe(false);
  expect(turn).toBe(2);
});

it('requires a real canonical sequence before calling the reducer effect shared',()=>{
  const proof=directEffectEvidence();
  for(const row of [...proof.effectBefore,...proof.effectAfter])delete row.journal[0].sequence;
  expect(directEffectOutcome(proof)).toBe(false);
});


it('starts continuous observation from an established effect despite later bootstrap activity',async()=>{
  const proof=directEffectEvidence(),order=[],witnesses=[];
  const activity=proof.effectBefore.map(row=>({capacity:256,entries:[{tick:2},...row.events,{tick:5099,category:'connection'}]}));
  await startDirectEffectWitness(proof,async()=>{
    order.push('start');
    for(const state of activity){const witness=createEffectWitness(proof.effectRequest);witness.sample(state,0);witnesses.push(witness);}
  },async()=>{order.push('read');return proof.effectBefore.map((row,index)=>({...row,continuous:witnesses[index].read()}));});
  expect(order).toEqual(['start','read']);expect(directEffectOutcome(proof)).toBe(true);
  expect(proof.effectBaseline.every(row=>!row.continuous)).toBe(true);
  const later={...activity[0],entries:[...activity[0].entries.slice(0,-1),{tick:401,category:'connection'},activity[0].entries.at(-1)]};
  witnesses[0].sample(later,50);
  proof.effectAfter[0].continuous=witnesses[0].read();
  expect(proof.effectAfter[0].continuous.error).toBeNull();
  expect(directEffectOutcome(proof)).toBe(true);
});
it('refuses to start without two actual baseline effects and rejects a changed initial witness',async()=>{
  const missing=directEffectEvidence();missing.effectBefore[1].events=[];
  let started=false;
  await expect(startDirectEffectWitness(missing,async()=>{started=true;},async()=>[])).rejects.toThrow('Missing actual effect baseline');
  expect(started).toBe(false);
  const proof=directEffectEvidence(),changed=structuredClone(proof.effectBefore);changed[0].events.push(changed[0].events[0]);
  await expect(startDirectEffectWitness(proof,async()=>{},async()=>changed)).rejects.toThrow('Effect baseline changed');
  const absent=directEffectEvidence();
  await expect(startDirectEffectWitness(absent,async()=>{},async()=>absent.effectBefore)).rejects.toThrow('Initial continuous witness');
});


describe('replacement browser evaluation budget', () => {
  it('defaults replacement evaluations to 180 seconds beyond the longest 120-second phase', () => {
    const args = ['--out', 'target/options-only', '--routes', 'automatic-fallback'];
    expect(optionsFrom(recoveryMatrixArgs(args, 'replacement')).timeout).toBe(180);
    expect(args).not.toContain('--timeout');
  });
  it('retains an explicit evaluation bound in the matrix options', () => {
    const args = ['--out', 'target/options-only', '--timeout', '150'];
    expect(optionsFrom(recoveryMatrixArgs(args, 'replacement')).timeout).toBe(150);
    expect(recoveryMatrixArgs(args, 'replacement')).toEqual(args);
  });
  it('keeps other recovery cases on the ordinary matrix default', () => {
    expect(optionsFrom(recoveryMatrixArgs(['--out', 'target/options-only'], 'divergence')).timeout).toBe(90);
  });
});
