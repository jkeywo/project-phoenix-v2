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
