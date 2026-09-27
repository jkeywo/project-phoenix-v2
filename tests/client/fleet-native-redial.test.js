import {describe,it,expect} from 'vitest';
import {nativeGmRedialOutcome} from '../../scripts/fleet-native-redial.mjs';
const labels=['ship-1','ship-2','ship-3','ship-4','gm-1','gm-2'];
function evidence(){
 const before=labels.map((label,i)=>({label,slot:i+1,adopted:true,commands:i<4?[{origin:i+1,seq:1,tick:600,ship:label}]:[]}));
 const after=before.map(peer=>({...structuredClone(peer),errors:[],recovery:{losses:[{slot:6,tick:603}]},
   commands:[...peer.commands,...(peer.slot<5?[{origin:peer.slot,seq:2,tick:950,ship:peer.label}]:[])],
   frames:[900,1200].map(tick=>({t:'digest',d:{from:peer.slot,tick,digest:'1234567890abcdef'}}))}));
 return {before,after,atTick:600,processBefore:{pid:123,startedAt:'start',alive:true},processAfter:{pid:123,startedAt:'start',alive:true},
   trigger:{generation:0,memberCreations:1},closed:{generation:0},opened:{generation:1,event:'open'},openRequest:{generation:1},
   identity:{sameCredential:true,sameOperator:true,sameSlot:true},
   commits:labels.map(()=>({id:7,kind:'reconnect',candidate:{host:6,operator_id:'gm-2'},tick:650,digest:42})),
   resume:{accepted:true},resumeApplied:true};
}
describe('native GM socket redial proof',()=>{
 it('requires preserved process/capability, actual new transport, canonical commits and post-commit digests',()=>{
   expect(nativeGmRedialOutcome(evidence()).passed).toBe(true);
   for(const change of [
    e=>e.processAfter.pid++,e=>e.processAfter.alive=false,e=>e.identity.sameCredential=false,
    e=>e.trigger.memberCreations++,e=>e.opened.generation=0,e=>e.closed=null,
    e=>e.commits[5]=null,e=>e.commits[0].kind='first-time',e=>e.commits[0].candidate.host=5,
    e=>e.after[0].recovery.losses=[],e=>e.resumeApplied=false,
    e=>e.after[0].frames[1].d.digest='abcdef1234567890',e=>e.after[0].frames.pop(),
    e=>e.after[0].commands[1].ship='reset',e=>e.after[0].commands[1].seq=1,
   ]){const e=evidence();change(e);expect(nativeGmRedialOutcome(e).passed).toBe(false);}
 });
 it('rejects missing initial observations instead of throwing while the real projection is pending',()=>{
   expect(nativeGmRedialOutcome({before:evidence().before,after:[]}).passed).toBe(false);
 });
});
