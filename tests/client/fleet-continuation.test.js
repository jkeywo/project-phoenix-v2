import {describe,it,expect} from 'vitest';
import {createContinuationJournal,reconcileContinuation} from '../../gui/fleet-continuation.js';
const participants=[1,2,3,4,5,6];
const fleet=()=>participants.map(local=>createContinuationJournal({local,participants}));
const frame=label=>JSON.stringify({label});
const send=(hosts,from,raw,recipients=participants.filter(slot=>slot!==from))=>{
  const envelope=hosts[from-1].record(raw);
  for(const slot of recipients)hosts[slot-1].receive(envelope,from);
  return envelope;
};
describe('bounded owner-loss stream reconciliation',()=>{
  it('replays only the owner suffix omitted from each survivor, without duplicate effects',()=>{
    const hosts=fleet();
    send(hosts,1,frame('complete'));
    send(hosts,1,frame('last-command'),[2,3]);
    send(hosts,1,frame('last-gm-grant'),[2]);
    const tails=hosts.slice(1).map(host=>host.hold());
    const plan=reconcileContinuation({participants,departed:1,tails});
    const delivered=[];
    for(const host of hosts.slice(1)){
      const local=host.tail().local,applied=[];
      expect(host.replay(plan,raw=>applied.push(JSON.parse(raw).label))).toEqual(plan.frontier);
      delivered.push({local,applied});
    }
    expect(delivered).toEqual([
      {local:2,applied:[]},{local:3,applied:['last-gm-grant']},
      ...[4,5,6].map(local=>({local,applied:['last-command','last-gm-grant']})),
    ]);
    expect(()=>hosts[3].replay(plan,()=>{throw Error('must not replay');})).toThrow('stale-continuation-plan');
    for(const host of hosts.slice(1))host.commit(plan);
    const next=hosts[1].record(frame('post-migration'));
    expect(next.sequence).toBe(1);
    for(const host of hosts.slice(2))expect(host.receive(next,2)).toBe(frame('post-migration'));
    expect(hosts.slice(1).every(host=>host.tail().rows.every(row=>row.origin!==1))).toBe(true);
  });
  it('compacts only after every peer acknowledges, and still rejects replay after compaction',()=>{
    const hosts=fleet(),original=send(hosts,1,frame('first'));
    expect(hosts[1].retained.frames).toBe(1);
    for(let from=2;from<=6;from++)send(hosts,from,frame('ack'));
    expect(hosts[1].tail().rows.some(row=>row.origin===1)).toBe(false);
    expect(hosts[1].receive(original,1)).toBe(null);
  });
  it('refuses a missing survivor or missing owner frame rather than resume from a guessed prefix',()=>{
    const hosts=fleet();send(hosts,1,frame('partial'),[2]);
    const tails=hosts.slice(1).map(host=>host.hold());
    expect(()=>reconcileContinuation({participants,departed:1,tails:tails.slice(1)})).toThrow('incomplete-continuation-survivors');
    tails[0].rows=[];
    expect(()=>reconcileContinuation({participants,departed:1,tails})).toThrow('missing-continuation-frame');
  });
  it('rejects conflicting retained frames',()=>{
    const hosts=fleet();send(hosts,1,frame('partial'),[2,3]);
    const tails=hosts.slice(1).map(host=>host.hold());tails[1].rows[0].raw=frame('conflict');
    expect(()=>reconcileContinuation({participants,departed:1,tails})).toThrow('conflicting-continuation-frame');
  });
  it('checks origin and sequence before accepting delivery',()=>{
    const hosts=fleet(),first=hosts[0].record(frame('one'));
    expect(()=>hosts[1].receive(first,3)).toThrow('continuation-origin-mismatch');
    const second=hosts[0].record(frame('two'));
    expect(()=>hosts[1].receive(second,1)).toThrow('continuation-stream-gap');
    expect(hosts[1].failed).toBe('continuation-stream-gap');
  });
  it('fails permanently at the explicit tail bound instead of dropping needed history',()=>{
    const host=createContinuationJournal({local:1,participants,limits:{frames:1,bytes:1000}});
    host.record(frame('one'));
    expect(()=>host.record(frame('two'))).toThrow('continuation-tail-overflow');
    expect(()=>host.hold()).toThrow('continuation-tail-overflow');
  });
});
