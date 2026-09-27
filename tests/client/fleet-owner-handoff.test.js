import { describe, expect, it, vi } from 'vitest';
import { makeWorld, makePeerFactory, leadOn, memberOn, settle } from './fleet-session-harness.js';
import { transportLeversFromLocation } from '../../gui/transport-levers.js';
import { encodeHostFrame, simulationFrame, HOST_FRAME_TICK } from '../../gui/host-mesh.js';

const tick = (from, through) => encodeHostFrame(simulationFrame(HOST_FRAME_TICK,
  { from, ready_through: through, commands: [] }));

describe('authenticated fleet owner transport continuation', () => {
  it.each([['ws-relay','notice-first',false],['webrtc','notice-first',false],['webrtc','media-first',true],['ws-relay','notice-first',true]])('%s %s partial=%s rebinds the same code and surviving identities, with one simulation adoption per peer', async (route,order,partial) => {
    const world = makeWorld(), sockets = [], errors = [], stages = [];
    const factories = { socket: url => { const socket = world.socket(url); sockets.push(socket); return socket; }, peer: makePeerFactory() };
    const options = {
      transports: route === 'ws-relay' ? ['ws-relay'] : ['webrtc','ws-relay'],
      levers:transportLeversFromLocation(`?transport=${route}`),
      onContinuation: async request => {
        stages.push(request);
        return { status: {begin:'held',replayed:'replayed',commit:'committed'}[request.op], loss_tick:101 };
      },
      onContinuationFrame:()=>true,
      authenticateFrame:(raw,slot)=>JSON.parse(raw).d.from===slot,
      onError:(reason,detail)=>errors.push({reason,detail}),
    };
    const lead = await leadOn(world,factories,options);
    const code = lead.code.full;
    const two = await memberOn(world,factories,code,options);
    const three = await memberOn(world,factories,code,options);
    lead.fleet.freeze(); await settle();
    lead.fleet.broadcast(tick(1,100));
    two.member.broadcast(tick(2,100));
    three.member.broadcast(tick(3,100));
    await settle();
    expect(two.simulationFrames.length).toBe(2);
    expect(three.simulationFrames.length).toBe(2);
    if (partial) {
      const payload=JSON.stringify({continuation:'phoenix-fleet-continuation-v1',kind:'chunk',body:{index:0,count:2,text:'partial-owner-message'}});
      if(route==='ws-relay')for(const socket of sockets.slice(1))socket.onmessage({data:JSON.stringify({v:1,type:'relay',class:'reliable',payload})});
      else for(const channel of factories.peer.channels)if(channel.origin==='answer' && channel.label==='reliable')channel.send(payload);
      await settle();
    }
    sockets[0].onclose = null; // the lost process cannot schedule its own reconnect
    const closeMedia=()=>{
      if (route === 'webrtc') for (const channel of [...factories.peer.channels]) {
        if (channel.origin === 'answer') { channel.onclose = null; channel.onmessage = null; channel.close(); }
      }
    };
    if(order==='media-first'){closeMedia();await settle();sockets[0].close();}
    else {sockets[0].close();closeMedia();}
    for(let i=0;i<10;i++)await settle();
    expect(errors).toEqual([]);
    expect(two.member.isOwner).toBe(true);
    expect(()=>two.member.freeze()).not.toThrow();
    expect(two.member.completeGmJoin(999,'committed')).toBe(false);
    expect(two.member.code.full).toBe(code);
    expect(three.member.slot).toBe('slot-3');
    expect(stages.filter(stage=>stage.op==='commit')).toHaveLength(2);
    expect(two.simulationRosters).toHaveLength(1);
    expect(three.simulationRosters).toHaveLength(1);
    two.member.broadcast(tick(2,110)); await settle();
    expect(three.simulationFrames.at(-1)).toMatchObject({raw:tick(2,110),authSlot:2});
    two.member.close(); three.member.close();
  });
});



it('a fixed-slot replacement continues the authenticated stream counters', async () => {
  const world=makeWorld(), errors=[];
  const factories={socket:world.socket,peer:makePeerFactory()};
  const options={ transports:['ws-relay'], levers:transportLeversFromLocation('?transport=ws-relay'),
    onContinuation:async()=>({status:'held'}), onContinuationFrame:()=>true,
    onError:(reason,detail)=>errors.push({reason,detail}) };
  const lead=await leadOn(world,factories,options);
  const two=await memberOn(world,factories,lead.code.full,options);
  const three=await memberOn(world,factories,lead.code.full,options);
  lead.fleet.freeze();await settle();
  two.member.broadcast(tick(2,100));lead.fleet.broadcast(tick(1,100));await settle();
  two.member.close();await settle();
  const replacement=await memberOn(world,factories,lead.code.full,{...options,claim:'slot-2'});
  expect(replacement.member.slot).toBe('slot-2');
  replacement.member.broadcast(tick(2,110));lead.fleet.broadcast(tick(1,110));await settle();
  expect(errors).toEqual([]);
  expect(three.simulationFrames.at(-2)?.raw).toBe(tick(2,110));
  expect(replacement.simulationFrames.at(-1)?.raw).toBe(tick(1,110));
  lead.fleet.close();replacement.member.close();three.member.close();
});

it('never promotes on media failure while the service still reports the owner present',async()=>{
  vi.useFakeTimers();
  const world=makeWorld(),sockets=[],stages=[];
  const factories={socket:url=>{const socket=world.socket(url);if(sockets.length>=2){socket.sent=[];socket.send=raw=>socket.sent.push(JSON.parse(raw));}sockets.push(socket);return socket;},peer:makePeerFactory()};
  const options={transports:['webrtc'],levers:transportLeversFromLocation('?transport=webrtc'),
    onContinuation:async request=>{stages.push(request);return {status:'held'};},onContinuationFrame:()=>true};
  let lead,two;
  try {
    lead=await leadOn(world,factories,options);
    two=await memberOn(world,factories,lead.code.full,options);
    lead.fleet.freeze();await settle();
    for(const channel of factories.peer.channels)if(channel.origin==='answer'){
      channel.onclose=null;channel.onmessage=null;channel.close();
    }
    await settle();
    expect(sockets[1].readyState).toBe(1);
    expect(two.member.isOwner).toBe(false);
    expect(stages).toEqual([]);
    await vi.advanceTimersByTimeAsync(119999);await settle();
    expect(sockets[1].readyState).toBe(1);
    expect(two.member.isOwner).toBe(false);
    await vi.advanceTimersByTimeAsync(1);await settle();
    expect(sockets[1].readyState).toBe(3);
    expect(stages).toEqual([]);
    await vi.advanceTimersByTimeAsync(1000);await settle();
    expect(sockets.length).toBeGreaterThan(2);
    const retried=sockets.at(-1);
    // A subsequent relay closure must not be swallowed by stale continuationHold.
    retried.onmessage({data:JSON.stringify({v:1,type:'relay-closed',reason:'host-left'})});
    expect(retried.sent.some(frame=>frame.type==='fleet-state')).toBe(true);
  } finally {two?.member.close();lead?.fleet.close();vi.useRealTimers();}
});
