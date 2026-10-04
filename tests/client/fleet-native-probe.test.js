import { describe, expect, it } from 'vitest';
import vm from 'node:vm';
import { EventEmitter } from 'node:events';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { instrumentNativeFleetModule, nativeProbeOutcome, parseNativeProbeArgs, terminateNativeProbe, stageNativeProbeBundle } from '../../scripts/fleet-native-probe.mjs';

const args = ['--binary', 'host.exe', '--bundle', 'dist', '--source', '.', '--out', 'run', '--rendezvous', 'http://127.0.0.1:8788', '--origin', 'http://localhost:8080'];

describe('bounded native runtime bootstrap probe', () => {
  it('stages a self-contained host and client bundle by dereferencing directory links', () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'phoenix-probe-stage-'));
    try {
      const bundle = path.join(root, 'input'), assets = path.join(root, 'external');
      fs.mkdirSync(path.join(bundle, 'client'), {recursive:true});
      fs.mkdirSync(path.join(bundle, 'gui'));
      fs.mkdirSync(assets);
      fs.writeFileSync(path.join(assets, 'world.toml'), 'world');
      fs.writeFileSync(path.join(bundle, 'index.html'), 'host');
      fs.writeFileSync(path.join(bundle, 'client/index.html'), 'client');
      fs.writeFileSync(path.join(bundle, 'gui/host.js'), 'script');
      fs.writeFileSync(path.join(bundle, 'phoenix-hash.wasm'), 'wasm');
      for (const dir of [bundle, path.join(bundle, 'client')]) {
        fs.symlinkSync(assets, path.join(dir, 'assets'), process.platform === 'win32' ? 'junction' : 'dir');
      }
      const staged = path.join(root, 'staged');
      stageNativeProbeBundle(bundle, staged);
      fs.writeFileSync(path.join(assets, 'world.toml'), 'changed');
      for (const name of ['assets', 'client/assets']) {
        expect(fs.lstatSync(path.join(staged, name)).isSymbolicLink()).toBe(false);
        expect(fs.readFileSync(path.join(staged, name, 'world.toml'), 'utf8')).toBe('world');
      }
      for (const [name, value] of [['index.html','host'], ['client/index.html','client'], ['gui/host.js','script'], ['phoenix-hash.wasm','wasm']]) {
        expect(fs.readFileSync(path.join(staged,name),'utf8')).toBe(value);
      }
    } finally { fs.rmSync(root, {recursive:true,force:true}); }
  });
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

  it('closes the existing native GM socket once and proves identity without exposing the capability',async()=>{
    const reports=[],controls=[],sent=[];let poll,closed=0,members=0;
    const source=`export function createNativeFleetPeer(options) {
      globalThis.emitIdentity=()=>options.send({kind:'fleet_identity',identity:{operatorId:'gm-2',claim:'slot-6',reconnectCredential:'SECRET'}});
      options.createMember({factories:{socket:()=>({readyState:1,close(){globalThis.closeSocket();options.send({kind:'fleet_wire_close',generation:0});this.readyState=3;}})}});
      options.send({kind:'fleet_wire_send',generation:0,frame:'PRIVATE'});
      globalThis.emitIdentity();
      return {role:'gm',configure(){},update(){},receive(){},join(){throw new Error('must retain the existing member');}};
    }`;
    const context=vm.createContext({closeSocket:()=>closed++,window:{addEventListener(){}},navigator:{userAgent:'test'},
      setInterval:fn=>poll=fn,
      fetch:url=>{if(url.endsWith('/fleet-control'))return Promise.resolve({ok:true,json:async()=>controls.shift()||null});
        reports.push(JSON.parse(new URL(url).searchParams.get('event')));return Promise.resolve({ok:true});}});
    vm.runInContext(instrumentNativeFleetModule(source,'http://localhost/token',{recoveryControl:true})
      .replace('export function createNativeFleetPeer','function createNativeFleetPeer'),context);
    const peer=context.createNativeFleetPeer({send:record=>sent.push(record),createMember:options=>{members++;options.factories.socket();}});
    controls.push({kind:'gm-redial'});await poll();
    expect(closed).toBe(1);expect(members).toBe(1);
    expect(reports.find(row=>row.kind==='redial-requested').value).toEqual({generation:0,memberCreations:1});
    expect(sent).toContainEqual({kind:'fleet_wire_close',generation:0});
    peer.receive(JSON.stringify({native_wire:{event:'open',generation:1}}));context.emitIdentity();
    expect(reports.find(row=>row.kind==='fleet-wire-event').value).toEqual({event:'open',generation:1});
    expect(reports.find(row=>row.kind==='redial-identity').value).toEqual({sameCredential:true,sameOperator:true,sameSlot:true});
    controls.push({kind:'gm-redial'});await poll();
    expect(closed).toBe(1);expect(reports.some(row=>row.kind==='observer-error')).toBe(true);
    expect(JSON.stringify(reports)).not.toMatch(/SECRET|PRIVATE/);
  });

  it('refuses ambiguous source rewriting', () => {
    expect(() => instrumentNativeFleetModule('export const changed = true;', 'http://localhost')).toThrow();
    expect(() => instrumentNativeFleetModule('export function createNativeFleetPeer() {}\nexport function createNativeFleetPeer() {}', 'http://localhost')).toThrow();
  });
  it('holds two-stage claims until scheduled and changes exactly one authenticated incoming command when armed',async()=>{
    const reports=[],calls=[],controls=[],timers=[];let poll,receiver;
    const source=`export function createNativeFleetPeer(options) { globalThis.receive=options.send;
      return {configure(){},update(){},join(...args){globalThis.joinCalls.push(args);return true;}}; }`;
    const context=vm.createContext({joinCalls:calls,window:{addEventListener(){}},navigator:{userAgent:'test'},
      setInterval:fn=>{poll=fn;},setTimeout:fn=>{timers.push(fn);},
      fetch:url=>{if(url.endsWith('/fleet-control'))return Promise.resolve({ok:true,json:async()=>controls.shift()||null});
        reports.push(JSON.parse(new URL(url).searchParams.get('event')));return Promise.resolve({ok:true});}});
    vm.runInContext(instrumentNativeFleetModule(source,'http://localhost/token',{claim:'slot-3',deferJoin:true,recoveryControl:true})
      .replace('export function createNativeFleetPeer','function createNativeFleetPeer'),context);
    const received=[],peer=context.createNativeFleetPeer({send:row=>received.push(row)});
    expect(peer.join('secret-code',{},null,'ship')).toBe(true);expect(calls).toHaveLength(0);
    for(const attempt of ['race','challenge']){controls.push({kind:'join',startAt:Date.now()+1000,attempt});await poll();expect(timers).toHaveLength(1);timers.shift()();}
    expect(calls).toHaveLength(2);expect(calls[0][2].claim).toBe('slot-3');
    expect(reports.filter(row=>row.kind==='join-call').map(row=>row.value.attempt)).toEqual(['race','challenge']);
    controls.push({kind:'diverge'});await poll();receiver=context.receive;
    const input=()=>({kind:'fleet_frame',authenticated_slot:1,frame:JSON.stringify({t:'tick',d:{from:1,commands:[{origin:1,seq:3,tick:900,ship:'same-ship',payload:{type:'SetBoost',data:{active:true}}}]}})});
    receiver(input());receiver(input());
    expect(JSON.parse(received[0].frame).d.commands[0].payload.data.active).toBe(false);
    expect(JSON.parse(received[1].frame).d.commands[0].payload.data.active).toBe(true);
    expect(reports.filter(row=>row.kind==='divergence-injected')).toHaveLength(1);
    expect(JSON.stringify(reports)).not.toContain('secret-code');
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
