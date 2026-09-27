import {describe,it,expect} from 'vitest';
import {nativeRecoveryPeer,mixedRecoveryOutcome,mixedReplacementOutcome,mixedDivergenceOutcome,mixedFailureHook,mixedRecoveryOptions,main} from '../../scripts/fleet-mixed-recovery.mjs';
const labels=['ship-1','ship-2','ship-3','ship-4','gm-1','gm-2'];
const digest=(slot,tick,value='0123456789abcdef')=>({t:'digest',d:{from:slot,tick,digest:value}});
function fixture(failure='ship') {
 const victimLabel={ship:'ship-3',replacement:'ship-3',gm:'gm-2',leader:'ship-1'}[failure];
 const before=labels.map((label,index)=>({label,slot:index+1,adopted:true,runtime:['ship-3','ship-4','gm-2'].includes(label)?'native':'browser',
  commands:label.startsWith('ship')?[{origin:index+1,seq:1,tick:500,ship:'ship-'+(index+1)}]:[],recovery:{ships:[1,2,3,4].map(slot=>({slot,crewed:true})),losses:[]}}));
 const victim=before.find(peer=>peer.label===victimLabel),lossTick=900;
 const after=before.filter(peer=>peer!==victim).map(peer=>({...structuredClone(peer),phase:'InProgress',mesh:{tick:1800,peers:before.filter(p=>p!==victim&&p!==peer).map(p=>p.slot)},
  recovery:{ships:[1,2,3,4].map(slot=>({slot,crewed:slot!==victim.slot})),losses:[{slot:victim.slot,tick:lossTick}]},
  continuation:{status:'committed',loss_tick:lossTick},frames:[digest(peer.slot,1200),digest(peer.slot,1500)],controlFrames:[],errors:[]}));
 return {failure,before,after,victim:{label:victimLabel,slot:victim.slot},stopped:{confirmed:true},atTick:600};
}
describe('mixed recovery evidence',()=>{
 it.each(['ship','gm','leader'])('accepts complete actual %s observations',failure=>{expect(mixedRecoveryOutcome(fixture(failure)).passed).toBe(true);});
 it('requires native applied loss and observed victim exit, not a transport disconnect',()=>{const e=fixture();e.after.find(p=>p.runtime==='native').recovery.losses=[];e.stopped.confirmed=false;expect(mixedRecoveryOutcome(e).reasons).toEqual(expect.arrayContaining(['victim process/page exit unverified','one identical applied HostLoss per survivor required']));});
 it('rejects duplicate applied loss or command order',()=>{const e=fixture();e.after[0].recovery.losses.push({...e.after[0].recovery.losses[0]});e.after[0].commands.push({...e.after[0].commands[0]});const result=mixedRecoveryOutcome(e);expect(result.passed).toBe(false);expect(result.duplicateOrders).toHaveLength(1);});
 it('requires lost ship Backfill but does not invent a GM ship',()=>{const e=fixture();e.after[0].recovery.ships.find(s=>s.slot===e.victim.slot).crewed=true;expect(mixedRecoveryOutcome(e).passed).toBe(false);expect(mixedRecoveryOutcome(fixture('gm')).backfillApplicable).toBe(false);});
 it('rejects contradictory same-peer checkpoints, malformed digests and changed survivor identity',()=>{for(const mutation of [e=>e.after[0].frames.push(digest(e.after[0].slot,1200,'ffffffffffffffff')),e=>e.after[0].frames[0].d.digest=123,e=>e.after[0].slot=99]){const e=fixture();mutation(e);expect(mixedRecoveryOutcome(e).passed).toBe(false);}});
 it('requires continuation commit on native and browser survivors of owner loss',()=>{const e=fixture('leader');e.after.find(p=>p.runtime==='native').continuation.status='held';expect(mixedRecoveryOutcome(e).passed).toBe(false);});
 it('projects native accepted slot, diagnostics and actual digests without synthesizing phase/tick',()=>{const rows=[{kind:'simulation-roster',value:{generation:2,local:7}},{kind:'state',value:{roster_result:{generation:2,accepted:true},recovery:{losses:[{slot:1,tick:90}]},continuation:{status:'held'}}},{kind:'digest',value:{from:7,tick:100,digest:'0123456789abcdef'}}];const peer=nativeRecoveryPeer('ship-3',rows);expect(peer.slot).toBe(7);expect(peer.adopted).toBe(true);expect(peer.mesh).toBeUndefined();expect(peer.phase).toBeUndefined();expect(peer.recovery.losses).toEqual([{slot:1,tick:90}]);expect(nativeRecoveryPeer('ship-3',rows.filter(r=>r.kind!=='state')).adopted).toBe(false);});
 it('bounds scenarios and requires source receipts/rendering before runtime work',async()=>{expect(()=>mixedFailureHook('unknown')).toThrow();expect(()=>mixedFailureHook('ship',{faultSeconds:601})).toThrow();await expect(main(['--failure','ship'])).rejects.toThrow('build-receipt');});
 it('requires same-slot restored boundary, ship identity and one authoritative claim',()=>{
  const e=fixture('replacement');e.outcome=mixedRecoveryOutcome(e);
  const replacement={label:'ship-replacement',runtime:'native',adopted:true,slot:3,commands:[{origin:3,seq:2,tick:1600,ship:'ship-3'}],errors:[],frames:[digest(3,1800),digest(3,2100)],controlFrames:[]};
  e.afterReplacement=[...structuredClone(e.after),replacement];
  for(const peer of e.afterReplacement){peer.frames=[digest(peer.slot,1800),digest(peer.slot,2100)];peer.recovery={replacement:{slot:3,leader:1,boundary_tick:1500,claim_seq:1,result:peer===replacement?'recovered':peer.slot===1?'led':'witnessed'}};}
  e.afterReplacement[0].controlFrames=[{t:'slot-claim',d:{from:1,slot:3,claim_seq:1}}];
  expect(mixedReplacementOutcome(e).passed).toBe(true);
  e.raceRequired=true;e.winner='ship-replacement';replacement.routes=['ws-relay'];
  e.race=[{label:'ship-replacement',admitted:true,attempt:{accepted:true,startedMs:1000}},
    {label:'loser',refused:true,attempt:{accepted:true,startedMs:1050}}];
  e.challengeTick=1600;e.challenger={label:'loser',refused:true,attempt:{accepted:true,startedMs:2000}};
  expect(mixedReplacementOutcome(e).passed).toBe(true);
  e.challenger.refused=false;expect(mixedReplacementOutcome(e).passed).toBe(false);e.challenger.refused=true;
  e.race[1].attempt.startedMs=1500;expect(mixedReplacementOutcome(e).passed).toBe(false);e.race[1].attempt.startedMs=1050;
  replacement.commands[0].seq=1;expect(mixedReplacementOutcome(e).duplicateOrders).toHaveLength(1);replacement.commands[0].seq=2;
  replacement.commands[0].ship='reset-ship';expect(mixedReplacementOutcome(e).passed).toBe(false);
  replacement.commands[0].ship='ship-3';e.afterReplacement[0].controlFrames.push(e.afterReplacement[0].controlFrames[0]);expect(mixedReplacementOutcome(e).passed).toBe(false);
 });
});

it('requires native restored state, exact agreement, original ship controls and continuous actual effects together',()=>{
 const before=fixture().before,after=before.map(peer=>({...structuredClone(peer),frames:[digest(peer.slot,1500),digest(peer.slot,1800)],
   commands:peer.label.startsWith('ship-')?[...peer.commands,{origin:peer.slot,seq:2,tick:1600,ship:peer.commands[0].ship}]:[],
   recovery:{divergence:{leader:1,boundary_tick:1200,result:peer.slot===6?'recovered':peer.slot===1?'led':'witnessed'}},errors:[]}));
 const effectRequest={entity:'ship-1',correlation:'effect',amount_milli_hp:5000};
 const effects=['gm-1','gm-2'].map(label=>{
   const events=[{tick:400,category:'damage',detail:{type:'damage',data:{weapon:'gm.direct',amount:5,hull_damage:5}},links:[{role:'victim',entity:{entity_id:'ship-1'}}]}];
   return {label,oldestTick:5,events,journal:[{correlation:'effect',action_kind:'direct-effect',tick:400,sequence:1,outcome:'applied'}],
     continuous:{error:null,samples:10,throughTick:1800,events:structuredClone(events)}};
 });
 const e={before,after,victim:'gm-2',injected:{from:1,authenticatedSlot:1,tick:800},effectRequest,effectWitnessRequired:true,effectBefore:effects,effectAfter:structuredClone(effects)};
 expect(mixedDivergenceOutcome(e).passed).toBe(true);
 for(const mutate of [v=>v.injected.authenticatedSlot=2,v=>v.after[0].frames[0].d.digest='ffffffffffffffff',
   v=>v.after[1].commands[1].ship='reset',v=>v.effectAfter[1].continuous.error='sampling-gap']){
   const bad=structuredClone(e);mutate(bad);expect(mixedDivergenceOutcome(bad).passed).toBe(false);
 }
});

it('parses opt-in post-fault windows without consuming matrix options or changing defaults',()=>{
 expect(mixedRecoveryOptions(['--failure','ship','--render']).faultSeconds).toBe(90);
 expect(mixedRecoveryOptions(['--fault-seconds','600','--failure','leader','--deadline','900','--render'])).toEqual({failure:'leader',faultSeconds:600,args:['--deadline','900','--render']});
 for(const value of ['0','601','1.5','NaN'])expect(()=>mixedRecoveryOptions(['--failure','ship','--fault-seconds',value])).toThrow();
 expect(()=>mixedRecoveryOptions(['--failure','ship','--fault-seconds'])).toThrow();
 expect(()=>mixedRecoveryOptions(['--failure','ship','--fault-seconds','60','--fault-seconds','90'])).toThrow();
});
