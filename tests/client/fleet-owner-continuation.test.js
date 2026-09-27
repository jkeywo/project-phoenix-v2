import { describe, expect, it } from 'vitest';
import { createOwnerContinuation } from '../../gui/fleet-owner-continuation.js';

describe('owner continuation transaction', () => {
  it('reconciles one partially relayed command, waits for every replay ack, and commits once', async () => {
    const peers = new Map(), wire = [], delivered = [], committed = [], requests = [];
    for (const local of [1, 2, 3]) peers.set(local, createOwnerContinuation({
      local, owner: 1, participants: [1, 2, 3],
      request: async request => {
        requests.push([local, request]);
        return {status: {begin:'held',replayed:'replayed',commit:'committed'}[request.op], loss_tick: 101};
      },
      deliver: () => {},
      replayFrame: (raw, source) => delivered.push([local, raw, source]),
      send: (target, raw) => wire.push([local, target, JSON.parse(raw)]),
      onCommit: result => committed.push([local, result]),
      onError: (_, detail) => { throw new Error(detail); },
    }));
    const envelope = JSON.parse(peers.get(1).broadcast('one-owner-command'));
    expect(peers.get(2).receive(envelope.body, 1)).toBe('one-owner-command');
    await Promise.all([peers.get(2).begin(1), peers.get(3).begin(1)]);
    await peers.get(2).connected();
    expect(committed).toEqual([]);
    await peers.get(3).connected();
    while (wire.length) {
      const [source, target, message] = wire.shift();
      await peers.get(target).control(message.kind, message.body, source);
    }
    expect(delivered).toEqual([[3, 'one-owner-command', 1]]);
    expect(committed.map(([local])=>local).sort()).toEqual([2,3]);
    expect(peers.get(2).owner).toBe(2);
    expect(peers.get(3).owner).toBe(2);
    expect(requests.filter(([,request])=>request.op==='commit').every(([,request])=>JSON.stringify(request.acked)==='[2,3]')).toBe(true);
  });

  it('keeps every survivor held when replay watermarks disagree', async () => {
    const peers = new Map(), wire = [], errors = [];
    for (const local of [2,3]) peers.set(local, createOwnerContinuation({
      local, owner:1, participants:[1,2,3],
      request: async request => ({status: {begin:'held',replayed:'replayed',commit:'committed'}[request.op], loss_tick: local}),
      deliver:()=>{}, send:(target,raw)=>wire.push([local,target,JSON.parse(raw)]),
      onError:(_,detail)=>errors.push(detail),
    }));
    await Promise.all([...peers.values()].map(peer=>peer.begin(1)));
    for (const peer of peers.values()) await peer.connected();
    while(wire.length){const[source,target,message]=wire.shift();await peers.get(target).control(message.kind,message.body,source);}
    expect(errors).toEqual(['continuation-watermark-disagreement']);
    expect([...peers.values()].every(peer=>peer.phase!=='live')).toBe(true);
  });
});
it('never acknowledges replay or commits after the core rejects a retained row', async () => {
  const peers=new Map(), wire=[], stages=[], errors=[];
  for(const local of [1,2,3])peers.set(local,createOwnerContinuation({
    local,owner:1,participants:[1,2,3],
    request:async request=>{stages.push([local,request.op]);return {status:{begin:'held',replayed:'replayed',commit:'committed'}[request.op],loss_tick:101};},
    replayFrame:()=>false,deliver:()=>{},send:(target,raw)=>wire.push([local,target,JSON.parse(raw)]),
    onError:(_,detail)=>errors.push(detail),
  }));
  const frame=JSON.parse(peers.get(1).broadcast('retained-row'));
  peers.get(2).receive(frame.body,1);
  await peers.get(2).begin(1);await peers.get(3).begin(1);
  await peers.get(2).connected();await peers.get(3).connected();
  while(wire.length){const[source,target,message]=wire.shift();await peers.get(target).control(message.kind,message.body,source);}
  expect(errors).toContain('continuation-replay-ingress-refused');
  expect(stages).not.toContainEqual([3,'replayed']);
  expect(stages.some(([,op])=>op==='commit')).toBe(false);
});

it('buffers resumed egress and fans out early peer streams across a delayed coordinator commit acknowledgement', async () => {
  const participants=[1,2,3,4],peers=new Map(),wire=[],errors=[],delivered=[],pending=[];
  let resolveCoordinator;
  const coordinatorCommit=new Promise(resolve=>{resolveCoordinator=resolve;});
  for(const local of participants.slice(1))peers.set(local,createOwnerContinuation({
    local,owner:1,participants,
    request:async request=>request.op==='commit'&&local===2?coordinatorCommit:
      {status:{begin:'held',replayed:'replayed',commit:'committed'}[request.op],loss_tick:101},
    replayFrame:()=>true,deliver:raw=>delivered.push([local,raw]),
    send:(target,raw)=>wire.push([local,target,JSON.parse(raw)]),
    onError:(_,detail)=>errors.push(detail),
  }));
  const flush=async()=>{
    for(let round=0;round<20;round++){
      while(wire.length){
        const [source,target,message]=wire.shift();
        if(message.kind!=='stream'){pending.push(peers.get(target).control(message.kind,message.body,source));continue;}
        const deliverAndRelay=raw=>{
          delivered.push([target,raw]);
          if(target===2)for(const sibling of [3,4])if(sibling!==source)wire.push([2,sibling,message]);
        };
        const fresh=peers.get(target).receive(message.body,message.body.origin,deliverAndRelay);
        if(fresh)deliverAndRelay(fresh);
      }
      await Promise.resolve();
    }
  };
  await Promise.all([...peers.values()].map(peer=>peer.begin(1)));
  for(const peer of peers.values())await peer.connected();
  await flush();
  expect(peers.get(2).phase).toBe('replayed');
  expect(peers.get(3).phase).toBe('live');
  expect(peers.get(4).phase).toBe('live');
  expect(peers.get(2).broadcast('new-coordinator-tick')).toBeNull();
  const early=peers.get(3).broadcast('early-member-tick');
  wire.push([3,2,JSON.parse(early)]);await flush();
  expect(delivered).toEqual([]);
  expect(errors).toEqual([]);
  resolveCoordinator({status:'committed',loss_tick:101});
  await flush();await Promise.all(pending);await flush();
  expect(errors).toEqual([]);
  expect(delivered.filter(([target])=>target===2)).toEqual([[2,'early-member-tick']]);
  expect(delivered.filter(([target])=>target===3)).toEqual([[3,'new-coordinator-tick']]);
  expect(delivered.filter(([target])=>target===4)).toEqual([[4,'early-member-tick'],[4,'new-coordinator-tick']]);
});
