import { afterEach, expect, it, vi } from 'vitest';
import { createAudioAssetRevision } from '../../gui/audio-asset-revision.js';
const response = revision => ({ok:true,headers:{get:()=> 'application/json; charset=utf-8'},
  text:async()=>JSON.stringify({capability:'phoenix-native-asset-revision',version:1,revision})});
function rig(fetch) {
  vi.useFakeTimers();
  const root={setTimeout,clearTimeout,setInterval,clearInterval};
  const source=createAudioAssetRevision({root,fetch}),changes=[];
  const stop=source.watch(()=>changes.push(source.read()));
  return {root,source,changes,stop};
}
afterEach(()=>vi.useRealTimers());
it.each([
  {ok:false},
  {ok:true,headers:{get:()=> 'text/html'},text:async()=>'<html>fallback</html>'},
  {ok:true,headers:{get:()=> 'application/json'},text:async()=>'{"version":1,"revision":"1"}'},
  response('NaN'),response('-1'),response('01'),response('999999999999999999999'),
])('probes a non-capable delivery once, validates the capability, and never polls its fallback',async value=>{
  const fetch=vi.fn(async()=>value),state=rig(fetch);
  await vi.advanceTimersByTimeAsync(30000);
  expect(fetch).toHaveBeenCalledTimes(1);expect(state.changes).toEqual([]);state.stop();
});
it('polls only advertised native revisions, backs off failure, and retires discovery on local host takeover',async()=>{
  let revision='1',fail=false;
  const fetch=vi.fn(async()=>{if(fail)throw new Error('offline');return response(revision);});
  const state=rig(fetch);await vi.advanceTimersByTimeAsync(0);
  expect(state.changes).toEqual(['native:1']);
  await vi.advanceTimersByTimeAsync(1000);expect(state.changes).toEqual(['native:1']);
  revision='2';await vi.advanceTimersByTimeAsync(1000);expect(state.changes).toEqual(['native:1','native:2']);
  fail=true;await vi.advanceTimersByTimeAsync(1000);const calls=fetch.mock.calls.length;
  await vi.advanceTimersByTimeAsync(4999);expect(fetch).toHaveBeenCalledTimes(calls);
  fail=false;revision='3';await vi.advanceTimersByTimeAsync(1);expect(state.source.read()).toBe('native:3');
  state.root.__hostPackRevision=()=>7;await vi.advanceTimersByTimeAsync(100);expect(state.source.read()).toBe('local:7');
  const final=fetch.mock.calls.length;await vi.advanceTimersByTimeAsync(10000);expect(fetch).toHaveBeenCalledTimes(final);
  state.stop();expect(vi.getTimerCount()).toBe(0);
});
it('times out one discovery and ignores its late response after disposal',async()=>{
  let finish,signal;const fetch=vi.fn((_,options)=>{signal=options.signal;return new Promise(resolve=>{finish=resolve;});});
  const state=rig(fetch);await vi.advanceTimersByTimeAsync(2000);expect(signal.aborted).toBe(true);
  finish(response('7'));await vi.advanceTimersByTimeAsync(0);expect(state.changes).toEqual([]);
  await vi.advanceTimersByTimeAsync(10000);expect(fetch).toHaveBeenCalledTimes(1);state.stop();
  expect(vi.getTimerCount()).toBe(0);
});
it('stopping aborts an in-flight advertised request and suppresses its late callback',async()=>{
  let finish,signal;const fetch=vi.fn().mockResolvedValueOnce(response('1')).mockImplementation((_,options)=>{
    signal=options.signal;return new Promise(resolve=>{finish=resolve;});
  });
  const state=rig(fetch);await vi.advanceTimersByTimeAsync(1000);state.stop();expect(signal.aborted).toBe(true);
  finish(response('2'));await vi.advanceTimersByTimeAsync(0);expect(state.changes).toEqual(['native:1']);expect(vi.getTimerCount()).toBe(0);
});
