import {describe,it,expect,vi} from 'vitest';
import {makeWorld,makePeerFactory,leadOn,memberOn,settle} from './fleet-session-harness.js';
import {HOST_ROLE_GM,simulationFrame,encodeHostFrame} from '../../gui/host-mesh.js';
import {RELIABLE_CHANNEL} from '../../gui/rendezvous-transport.js';
import {transportLeversFromLocation} from '../../gui/transport-levers.js';
import {continuationEnvelope} from '../../gui/fleet-owner-continuation.js';
const tick=(from,n)=>encodeHostFrame(simulationFrame('tick',{from,watermark:n,commands:[]},n));
const frame=(type,from,n)=>encodeHostFrame(simulationFrame(type,{from,id:1},n));
describe('same-handle GM reconnect stream boundary',()=>{
 it.each(['direct','ws-relay'])('restores privately and resumes an owner-seeded stream after dropped rows on %s',async route=>{
  vi.useFakeTimers();let lead,gm;
  try {
   const world=makeWorld(),sockets=[];
   const factories={peer:makePeerFactory(),socket:url=>{const socket=world.socket(url);if(url.endsWith('/v1/join'))sockets.push(socket);return socket;}};
   const errors=[],begins=[];
   const common={onContinuation:async()=>({status:'held'}),onError:(reason,detail)=>errors.push({reason,detail}),
    authenticateFrame:(raw,slot)=>JSON.parse(raw).d?.from===slot};
   lead=await leadOn(world,factories,{...common,ship:{template_path:'lead.toml'},
    ...(route==='ws-relay'?{transports:['ws-relay']}:{}),
    credentialFactory:()=> 'private-gm-capability',onBeginGmJoin:request=>{begins.push(request);return true;}});
   gm=await memberOn(world,factories,lead.code.suffix,{...common,role:HOST_ROLE_GM,
    ...(route==='ws-relay'?{levers:transportLeversFromLocation('?transport=ws-relay')}:{})});
   lead.fleet.update({ready:true});lead.fleet.setCrewReadiness({connected:1,ready:1});lead.fleet.setStartValidation(true);
   gm.member.setGmReady(true);gm.member.setStartValidation(true);await settle();
   expect(lead.grants).toHaveLength(1);
   const identity=gm.member.reconnectCredential;
   gm.member.broadcast(tick(2,10));lead.fleet.broadcast(tick(1,10));await settle();
   expect(lead.simulationFrames.some(row=>row.raw===tick(2,10))).toBe(true);
   sockets.at(-1).close();
   if(route==='direct') factories.peer.channels.find(ch=>ch.origin==='offer'&&ch.label===RELIABLE_CHANNEL).close();
   // Both directions mint rows which this disconnected transport cannot carry.
   gm.member.broadcast(tick(2,11));gm.member.broadcast(tick(2,12));
   lead.fleet.broadcast(tick(1,11));lead.fleet.broadcast(tick(1,12));
   await settle();await vi.advanceTimersByTimeAsync(100);await settle();
   expect(gm.member.pendingGmJoin).toBe(true);expect(begins).toHaveLength(1);
   const pendingCount=lead.simulationFrames.length;
   gm.member.broadcast(tick(2,19));await settle();
   expect(errors).toEqual([]);expect(lead.simulationFrames).toHaveLength(pendingCount);
   const pause=frame('gm-join',1,20),snapshot=frame('snapshot',1,20);
   lead.fleet.broadcast(pause);lead.fleet.broadcast(snapshot);await settle();
   expect(gm.simulationFrames.some(row=>row.raw===pause)).toBe(true);
   expect(gm.simulationFrames.some(row=>row.raw===snapshot)).toBe(true);
   const count=lead.simulationFrames.length;
   // Even a syntactically authentic continuation envelope cannot bypass the
   // candidate lane and poison the owner's journal before restore commits.
   gm.member.broadcast(continuationEnvelope('stream',{origin:2,sequence:999,raw:tick(2,99),ack:{1:0,2:0}}));
   gm.member.broadcast(tick(2,20));await settle();
   expect(lead.simulationFrames).toHaveLength(count);expect(errors).toEqual([]);
   const proof=frame('gm-join',2,20);gm.member.broadcast(proof);await settle();
   expect(lead.simulationFrames.at(-1)).toEqual({raw:proof,authSlot:2});
   expect(gm.member.pendingGmJoin).toBe(true);
   expect(lead.fleet.completeGmJoin(begins[0].id,'committed')).toBe(true);await settle();
   expect(gm.member.reconnectCredential).toBe(identity);expect(gm.member.pendingGmJoin).toBe(false);
   expect(gm.simulationRosters).toHaveLength(1);
   gm.member.broadcast(tick(2,30));lead.fleet.broadcast(tick(1,30));await settle();
   expect(lead.simulationFrames.at(-1)).toEqual({raw:tick(2,30),authSlot:2});
   expect(gm.simulationFrames.at(-1)).toEqual({raw:tick(1,30),authSlot:1});
   expect(errors).toEqual([]);
  } finally {gm?.member.close();lead?.fleet.close();vi.clearAllTimers();vi.useRealTimers();}
 });
});
