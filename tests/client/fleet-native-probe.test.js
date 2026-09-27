import { describe, expect, it } from 'vitest';
import vm from 'node:vm';
import { instrumentNativeFleetModule, nativeProbeOutcome, parseNativeProbeArgs } from '../../scripts/fleet-native-probe.mjs';

const args = ['--binary', 'host.exe', '--bundle', 'dist', '--source', '.', '--out', 'run', '--rendezvous', 'http://127.0.0.1:8788', '--origin', 'http://localhost:8080'];

describe('bounded native runtime bootstrap probe', () => {
  it('requires concrete inputs and refuses missing, repeated and unbounded options', () => {
    expect(parseNativeProbeArgs(args).seconds).toBe(45);
    for (const extra of [['--seconds', '0'], ['--seconds', '301'], ['--seconds', '1.5'], ['--seconds', 'NaN'], ['--seconds'], ['--extra', 'x'], ['--out', 'other']]) {
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
      fetch: value => { messages.push(JSON.parse(new URL(value).searchParams.get('event'))); return Promise.resolve(); } };
    vm.createContext(context);
    vm.runInContext(instrumentNativeFleetModule(source, 'http://127.0.0.1/token').replace('export function createNativeFleetPeer', 'function createNativeFleetPeer'), context);
    const peer = context.createNativeFleetPeer({send: value => calls.push(value), onDiag: value => calls.push(value)});
    const config = {owner:false,stamp:'content',credentials:['SECRET']};
    expect(peer.configure(config)).toBe(config);
    const state = {crew:{connected:3,ready:3},frames:['PRIVATE']};
    expect(peer.update(state)).toBe(state);
    expect(calls).toHaveLength(4);
    expect(messages.find(row => row.kind === 'onDiag').value.transport).toBe('ws-relay');
    expect(JSON.stringify(messages)).not.toMatch(/SECRET|PRIVATE/);
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

  it('refuses ambiguous source rewriting', () => {
    expect(() => instrumentNativeFleetModule('export const changed = true;', 'http://localhost')).toThrow();
    expect(() => instrumentNativeFleetModule('export function createNativeFleetPeer() {}\nexport function createNativeFleetPeer() {}', 'http://localhost')).toThrow();
  });
});
