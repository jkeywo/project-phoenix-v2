import { describe, expect, it } from 'vitest';
import vm from 'node:vm';
import { EventEmitter } from 'node:events';
import { instrumentNativeFleetModule, nativeProbeOutcome, parseNativeProbeArgs, terminateNativeProbe } from '../../scripts/fleet-native-probe.mjs';

const args = ['--binary', 'host.exe', '--bundle', 'dist', '--source', '.', '--out', 'run', '--rendezvous', 'http://127.0.0.1:8788', '--origin', 'http://localhost:8080'];

describe('bounded native runtime bootstrap probe', () => {
  it('requires concrete inputs and refuses missing, repeated and unbounded options', () => {
    expect(parseNativeProbeArgs(args).seconds).toBe(45);
    for (const extra of [['--seconds', '0'], ['--seconds', '961'], ['--seconds', '1.5'], ['--seconds', 'NaN'], ['--seconds'], ['--extra', 'x'], ['--out', 'other']]) {
      expect(() => parseNativeProbeArgs([...args, ...extra])).toThrow();
    }
    expect(() => parseNativeProbeArgs(args.slice(2))).toThrow('--binary');
  });

  it('forwards actual adapter calls and records routes without logging private wire or reconnect identity', () => {
    const messages = [], calls = [];
    const source = `export function createNativeFleetPeer(options = {}) {
      return { configure(raw) { options.onDiag({event:'transport',transport:'ws-relay'}); return raw; },
        update(raw) { options.send({kind:'fleet_identity', identity:{credential:'SECRET'}});
          options.send({kind:'fleet_wire_send', frame:'PRIVATE'});
          options.send({kind:'fleet_join_status',status:'admitted'}); return raw; } };
    }`;
    const context = { window: {addEventListener() {}}, navigator: { userAgent: 'Real engine test double' }, Set, JSON,
      fetch: value => { messages.push(JSON.parse(new URL(value).searchParams.get('event'))); return Promise.resolve({ok:true}); } };
    vm.createContext(context);
    vm.runInContext(instrumentNativeFleetModule(source, 'http://127.0.0.1/token').replace('export function createNativeFleetPeer', 'function createNativeFleetPeer'), context);
    const peer = context.createNativeFleetPeer({send: value => calls.push(value), onDiag: value => calls.push(value)});
    const config = {owner:false,stamp:'content',credentials:['SECRET']};
    expect(peer.configure(config)).toBe(config);
    const state = {crew:{connected:3,ready:3},recovery:{losses:[{slot:2,tick:300}]},continuation_result:{status:{status:'held'}},frames:[JSON.stringify({t:'tick',d:{from:1,commands:[{origin:1,seq:2,tick:301,ship:'stable-ship',payload:{type:'SetThrust',secret:'PRIVATE'}}]}})]};
    expect(peer.update(state)).toBe(state);
    expect(calls).toHaveLength(4);
    expect(messages.find(row => row.kind === 'onDiag').value.transport).toBe('ws-relay');
    expect(JSON.stringify(messages)).not.toMatch(/SECRET|PRIVATE/);
    expect(messages.find(row=>row.kind==='recovery-command').value).toMatchObject({origin:1,seq:2,ship:'stable-ship',type:'SetThrust'});
    expect(messages.find(row=>row.kind==='state').value).toMatchObject({recovery:{losses:[{slot:2,tick:300}]},continuation:{status:'held'}});
  });

  it('never promotes configured capability, owner registration or early process exit into a matrix pass', () => {
    const events = [{kind:'engine'},{kind:'configuration'},{kind:'fleet_code'}];
    expect(nativeProbeOutcome(events, null)).toMatchObject({bootstrapObserved:true,sixPeerWorkloadPassed:false,observedRoutes:[]});
    expect(nativeProbeOutcome(events, 0).bootstrapObserved).toBe(false);
    expect(nativeProbeOutcome(events, null, 'member').bootstrapObserved).toBe(false);
    expect(nativeProbeOutcome([{kind:'engine'}, {kind:'configuration'}, {kind:'fleet_join_status',value:{status:'admitted'}}], null, 'member').bootstrapObserved).toBe(true);
    expect(nativeProbeOutcome(events.slice(1), null).bootstrapObserved).toBe(false);
    expect(nativeProbeOutcome([{kind:'engine'},{kind:'configuration'},{kind:'fleet_join_status',value:{status:'pending'}}], null).bootstrapObserved).toBe(false);
  });

  it('supplies a replacement claim through the existing join API without logging reconnect capabilities', () => {
    const events=[],calls=[];
    const source=`export function createNativeFleetPeer() { return {configure(){},update(){},join(...args){return args;}}; }`;
    const context={window:{addEventListener(){}},navigator:{userAgent:'test'},Set,JSON,
      fetch:url=>{events.push(JSON.parse(new URL(url).searchParams.get('event')));return Promise.resolve({ok:true});}};
    vm.createContext(context);
    vm.runInContext(instrumentNativeFleetModule(source,'http://localhost/token',{claim:'slot-3'}).replace('export function createNativeFleetPeer','function createNativeFleetPeer'),context);
    const peer=context.createNativeFleetPeer({send:record=>calls.push(record)});
    const result=peer.join('code',{}, {reconnectCredential:'SECRET'},'ship');
    expect(result[2]).toEqual({reconnectCredential:'SECRET',claim:'slot-3'});
    expect(JSON.stringify(events)).not.toContain('SECRET');
  });

  it('refuses ambiguous source rewriting', () => {
    expect(() => instrumentNativeFleetModule('export const changed = true;', 'http://localhost')).toThrow();
    expect(() => instrumentNativeFleetModule('export function createNativeFleetPeer() {}\nexport function createNativeFleetPeer() {}', 'http://localhost')).toThrow();
  });
});

describe('bounded native child cleanup',()=>{
  function child(terminate) {
    const process=new EventEmitter();process.pid=12345;process.exitCode=null;process.signalCode=null;
    process.finish=()=>{process.signalCode='SIGTERM';process.emit('exit');};
    process.kill=()=>{if(terminate)process.finish();return true;};return process;
  }
  it('observes normal exit without invoking tree termination or leaking listeners',async()=>{
    const process=child(true);let forced=false;
    const result=await terminateNativeProbe(process,{graceMs:2,forceMs:2,forceTree:()=>{forced=true;}});
    expect(result.cleanupExitObserved).toBe(true);expect(forced).toBe(false);expect(process.listenerCount('exit')).toBe(0);
  });
  it('forces only the spawned tree after grace and requires an observed exit',async()=>{
    const process=child(false),pids=[];
    const result=await terminateNativeProbe(process,{graceMs:1,forceMs:1,forceTree:pid=>{pids.push(pid);process.finish();return {status:0};}});
    expect(pids).toEqual([12345]);expect(result).toEqual({forcedCleanup:{status:0},cleanupExitObserved:true});expect(process.listenerCount('exit')).toBe(0);
  });
  it('fails cleanup honestly when even forced termination does not produce exit',async()=>{
    const process=child(false);
    const result=await terminateNativeProbe(process,{graceMs:1,forceMs:1,forceTree:()=>({status:1})});
    expect(result.cleanupExitObserved).toBe(false);expect(process.listenerCount('exit')).toBe(0);
  });
  it('allows an explicit 960-second lifetime while retaining the 45-second default',()=>{
    expect(parseNativeProbeArgs(args).seconds).toBe(45);
    expect(parseNativeProbeArgs([...args,'--seconds','960']).seconds).toBe(960);
    expect(()=>parseNativeProbeArgs([...args,'--seconds','961'])).toThrow();
  });
});
