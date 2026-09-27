import { describe, expect, it, vi } from 'vitest';
import { createEffectWitness } from '../../scripts/fleet-effect-witness.mjs';
import { directEffectOutcome } from '../../scripts/fleet-browser-recovery.mjs';

const row = (tick, id = tick) => ({tick, category:'connection', detail:{type:'connection',data:{id}}});
const damage = (tick=4) => ({tick,category:'damage',detail:{type:'damage',data:{weapon:'gm.direct',amount:5,hull_damage:5}},links:[{role:'victim',entity:{entity_id:'ship'}}]});
const feed = (entries, capacity=6) => ({capacity, entries});
const make = options => createEffectWitness({entity:'ship',...options});
function baseline(observer) {
  observer.sample(feed([row(1),row(2),row(3)]),0);
  return observer.sample(feed([row(1),row(2),row(3),damage(),row(4),row(5)]),50);
}
function evict(observer) {
  observer.sample(feed([damage(),row(4),row(5),row(6),row(7),row(8)]),100);
  return observer.sample(feed([row(6),row(7),row(8),row(9),row(10),row(11)]),150);
}

describe('continuous actual-effect witness',()=>{
  it('preserves one actual event through eviction using complete overlapping ticks',()=>{
    const observer=make();baseline(observer);
    expect(evict(observer)).toMatchObject({error:null,events:[damage()],samples:4,throughTick:11});
    const log=observer.finish();
    expect(log.trace.map(sample=>sample.removed)).toEqual([0,0,3,3]);
    expect(log.trace.flatMap(sample=>sample.added).filter(item=>item.category==='damage')).toEqual([damage()]);
  });
  it('does not count repeated identical projections as repeated reducer effects',()=>{
    const observer=make();baseline(observer);
    for(let at=100;at<500;at+=50)observer.sample(feed([row(1),row(2),row(3),damage(),row(4),row(5)]),at);
    expect(observer.read()).toMatchObject({error:null,events:[damage()]});
  });
  it('retains proof when row-level ring eviction leaves a witnessed tick suffix',()=>{
    const observer=make();baseline(observer);
    const current=observer.sample(feed([row(4),row(5),row(6),row(7),row(8),row(9)]),100);
    expect(current).toMatchObject({error:null,events:[damage()],throughTick:9});
    expect(observer.finish().trace.at(-1).removed).toBe(4);
  });
  it('counts equal same-tick occurrences separately',()=>{
    const observer=make();observer.sample(feed([row(1),damage()]),0);
    expect(observer.sample(feed([row(1),damage(),damage()]),50).error).toBe('duplicate-effect');
  });
  it('accepts partial eviction before the oldest tick itself advances',()=>{
    const observer=make();
    observer.sample(feed([row(1,'a'),row(1,'b'),row(2),damage(),row(4),row(5)]),0);
    expect(observer.sample(feed([row(1,'b'),row(2),damage(),row(4),row(5),row(6)]),50))
      .toMatchObject({error:null,events:[damage()]});
    expect(observer.finish().trace.at(-1).removed).toBe(1);
  });
  it('refuses removal from an oldest tick when the ring is not full',()=>{
    const observer=make();
    observer.sample(feed([row(1,'a'),row(1,'b'),row(2),row(3)]),0);
    expect(observer.sample(feed([row(1,'b'),row(2),row(3),row(4)]),50).error)
      .toBe('non-capacity-eviction');
  });
  it('rejects a later duplicate after the original event left the ring',()=>{
    const observer=make();baseline(observer);evict(observer);
    expect(observer.sample(feed([row(9),row(10),row(11),damage(12),row(13),row(14)]),200).error).toBe('duplicate-effect');
  });
  it('detects any target direct damage even when the second amount was clamped',()=>{
    const observer=make();baseline(observer);evict(observer);
    const duplicate=damage(12);duplicate.detail.data.amount=1;duplicate.detail.data.hull_damage=1;
    expect(observer.sample(feed([row(9),row(10),row(11),duplicate,row(13),row(14)]),200).error).toBe('duplicate-effect');
  });
  it.each([
    ['no overlap',[row(6),row(7)]],
    ['only the prior newest tick',[row(5),row(6)]],
    ['changed retained suffix of older tick',[row(4,'forged'),row(5),row(6),row(7),row(8),row(9)]],
    ['changed complete tick',[row(3,'changed'),damage(),row(4),row(5)]],
    ['backward oldest',[row(0),row(1),row(2)]],
    ['backward newest',[row(1),row(2),row(3)]],
    ['backdated new fact',[row(1),row(2,'late'),row(3),damage(),row(4),row(5)]],
  ])('rejects %s instead of guessing continuity',(_name,entries)=>{
    const observer=make();baseline(observer);
    expect(observer.sample(feed(entries),100).error).toBeTruthy();
    expect(evict(observer).error).toBeTruthy();
  });
  it('rejects removal within the latest tick, even if another equal row survives',()=>{
    const observer=make();observer.sample(feed([row(1),row(2),row(2)]),0);
    expect(observer.sample(feed([row(1),row(2),row(3)]),50).error).toBe('changed-or-partially-evicted-tick');
    expect(observer.finish().rejected).toMatchObject({at:50,
      previous:[row(1),row(2),row(2)],current:[row(1),row(2),row(3)]});
  });
  it('permits reordered additions to the current tick with exact multiplicity',()=>{
    const observer=make();observer.sample(feed([row(1),row(2,'z')]),0);
    expect(observer.sample(feed([row(1),row(2,'a'),row(2,'z')]),50).error).toBeNull();
    expect(observer.finish().trace[1].added).toEqual([row(2,'a')]);
  });
  it('does not confuse other ships or ordinary weapon damage with the witness',()=>{
    const observer=make(),other=damage(2),weapon=damage(3);
    other.links[0].entity.entity_id='other';weapon.detail.data.weapon='beam';
    expect(observer.sample(feed([row(1),other,weapon,damage()]),0)).toMatchObject({error:null,events:[damage()]});
  });
  it.each([
    [{maxGapMs:20},50,'sampling-gap-or-clock-rewind'],
    [{},-1,'sampling-gap-or-clock-rewind'],
    [{maxSamples:1},10,'observer-bound'],
    [{maxDurationMs:5},10,'observer-bound'],
    [{maxBytes:1},10,'observer-byte-bound'],
  ])('fails closed at configured observation bounds %j',(options,at,error)=>{
    const observer=make(options);observer.sample(feed([row(1)]),0);
    expect(observer.sample(feed([row(1)]),at).error).toBe(error);
  });
  it.each([{},feed([]),feed([row(2),row(1)]),feed([row(-1)]),feed([row(1.5)]),feed([row(1)],0),feed([row(1)],5000)])('rejects invalid activity %j',activity=>{
    expect(make().sample(activity,0).error).toBe('invalid-activity');
  });
  it('rejects feed capacity changes',()=>{
    const observer=make();observer.sample(feed([row(1)]),0);
    expect(observer.sample(feed([row(1)],7),50).error).toBe('capacity-changed');
  });
});

function proof() {
  const observers=[make(),make()];
  const effectBefore=observers.map((observer,index)=>({label:'gm-'+(index+1),capacity:6,oldestTick:1,
    events:[damage()],continuous:baseline(observer),journal:[{correlation:'once',action_kind:'direct-effect',tick:4,sequence:3,outcome:'applied'}]}));
  const effectAfter=observers.map((observer,index)=>({...effectBefore[index],oldestTick:6,events:[],continuous:evict(observer)}));
  return {effectRequest:{entity:'ship',correlation:'once',amount_milli_hp:5000},effectBefore,effectAfter,effectSamples:[],effectWitnessRequired:true};
}
it('accepts eviction only with continuous actual event proof from both GM replicas',()=>{
  expect(directEffectOutcome(proof())).toBe(true);
  const missing=proof();delete missing.effectAfter[0].continuous;
  expect(directEffectOutcome(missing)).toBe(false);
  const gap=proof();gap.effectAfter[0].continuous.error='sampling-gap-no-complete-overlap';
  expect(directEffectOutcome(gap)).toBe(false);
  const prior=proof();const broken=structuredClone(prior.effectBefore);broken[1].continuous.error='duplicate-effect';prior.effectSamples.push(broken);
  expect(directEffectOutcome(prior)).toBe(false);
});
it('still requires actual5HP effect and canonical shared journal order',()=>{
  const journalOnly=proof();journalOnly.effectAfter[0].continuous.events=[];
  expect(directEffectOutcome(journalOnly)).toBe(false);
  const wrong=proof();wrong.effectAfter[1].journal=[{...wrong.effectAfter[1].journal[0],sequence:4}];
  expect(directEffectOutcome(wrong)).toBe(false);
});
it('browser adapter samples continuously, reports a delayed callback, and stops its timer',()=>{
  vi.useFakeTimers();const saved=globalThis.window;let at=0;
  vi.spyOn(performance,'now').mockImplementation(()=>at);
  const activity=feed([row(1),row(2)]);
  try {
    globalThis.window={__hostGmActivityState:()=>activity};
    createEffectWitness({entity:'ship',observe:true});
    at=50;vi.advanceTimersByTime(50);
    expect(window.__recoveryEffectWitness.read().samples).toBe(3);
    at=2000;
    expect(window.__recoveryEffectWitness.stop().error).toBe('sampling-gap-or-clock-rewind');
    expect(vi.getTimerCount()).toBe(0);
  } finally {globalThis.window=saved;vi.restoreAllMocks();vi.useRealTimers();}
});
