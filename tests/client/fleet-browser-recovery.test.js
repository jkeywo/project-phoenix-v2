import { describe, expect, it } from 'vitest';
import { failureOutcome, divergenceOutcome, observeRecovery } from '../../scripts/fleet-browser-recovery.mjs';

function evidence() {
  const before = Array.from({length:6}, (_,i)=>({label:`peer-${i+1}`,mesh:{slot:i+1,tick:100}}));
  const after = before.slice(1).map(peer=>({...peer,phase:'InProgress',mesh:{...peer.mesh,tick:200,peers:before.slice(1).filter(row=>row!==peer).map(row=>row.mesh.slot)},frames:[150,180].map(tick=>({t:'digest',d:{from:peer.mesh.slot,tick,digest:'same'}}))}));
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
    const broken=evidence(); broken.after[3].frames[0].d.digest='different';
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
