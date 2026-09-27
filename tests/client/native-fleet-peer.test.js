import { describe, expect, it, vi } from 'vitest';
import { createNativeFleetPeer } from '../../gui/native-fleet-peer.js';

describe('native technical fleet peer', () => {
  it('health-only ticks cannot flood control frames while changed state and every edge still publish', () => {
    let options;
    const handle = {update:vi.fn(),setCrewReadiness:vi.fn(),setGmReady:vi.fn(),setStartValidation:vi.fn(),broadcast:vi.fn(),forceStart:vi.fn(),close:vi.fn()};
    const peer = createNativeFleetPeer({send:vi.fn(),createOwner:value => {options=value;return handle;}});
    peer.configure({base:'https://fleet.test',owner:true,credentials:['one']});
    const state={ship:{template_path:'ship'},ship_ready:true,crew:{connected:3,ready:2},station_ratings:[['helm','Std']],gm_ready:false,validation:true};
    for(let tick=0;tick<512;tick++) peer.update({...state,health:{tick},frames:[String(tick)]});
    for(const method of ['update','setCrewReadiness','setGmReady','setStartValidation']) expect(handle[method]).toHaveBeenCalledTimes(1);
    expect(handle.broadcast.mock.calls.map(args=>args[0])).toEqual(Array.from({length:512},(_,i)=>String(i)));
    peer.update({...state,crew:{connected:3,ready:3},validation:false,force_start:true});
    peer.update({...state,crew:{connected:3,ready:3},validation:false,force_start:true});
    expect(handle.setCrewReadiness).toHaveBeenCalledTimes(2);
    expect(handle.setStartValidation).toHaveBeenCalledTimes(2);
    expect(handle.forceStart).toHaveBeenCalledTimes(2);
    const adopted=options.onSimulationRoster({participants:[]});
    peer.update({...state,roster_result:{generation:1,accepted:true}});
    peer.update({...state,roster_result:{generation:1,accepted:true}});
    return expect(adopted).resolves.toBe(true);
  });
  it('retains reliable frames until the bridge socket handlers are registered', async () => {
    const received = [];
    let options;
    const peer = createNativeFleetPeer({ send: vi.fn(), createOwner: candidate => {
      options = candidate;
      return { close() {} };
    } });
    peer.receive('before-socket');
    peer.configure({ base: 'https://fleet.test', credentials: ['one'] });
    const socket = options.factories.socket();
    peer.receive('before-handlers');
    socket.onmessage = event => received.push(event.data);
    await Promise.resolve();
    peer.receive('after-open');
    expect(received).toEqual(['before-socket', 'before-handlers', 'after-open']);
  });

  it('refuses an overflowing startup wire explicitly instead of losing reliable frames', () => {
    const send = vi.fn();
    const peer = createNativeFleetPeer({ send });
    for (let i = 0; i < 65; i += 1) peer.receive(`frame-${i}`);
    expect(send).toHaveBeenCalledExactlyOnceWith({
      kind: 'fleet_fault', reason: 'pending-frame-overflow', detail: '',
    });
  });
  it('admits a selected native ship and replays its control snapshot without a later Rust update', async () => {
    const sent = [];
    let options;
    const handle = {
      role: 'ship', update: vi.fn(), setCrewReadiness: vi.fn(),
      setGmReady: vi.fn(), setStartValidation: vi.fn(), broadcast: vi.fn(),
    };
    const peer = createNativeFleetPeer({
      send: record => sent.push(record),
      createMember: candidate => { options = candidate; return handle; },
    });
    peer.configure({ base: 'https://fleet.test', owner: false,
      ship_path: 'assets/entities/alliance_cruiser.toml', stamp:'4/phoenix-base/1',stamp_valid:true,credentials: ['configuration'] });
    expect(peer.join('SERVER-CODE', {}, null, 'ship')).toBe(true);
    expect(options.role).toBe('ship');
    expect(options.stamp).toBe('4/phoenix-base/1');
    expect(options.ship.template_path).toBe('assets/entities/alliance_cruiser.toml');
    expect(options.transports).toEqual(['ws-relay']);
    if (options.role === 'ship') expect(options.levers).toMatchObject({mode:'ws-relay',wsRelay:'only',pinned:true});
    peer.update({ ship: options.ship, ship_ready: true,
      crew: { connected: 3, ready: 3 }, station_ratings: [['helm', 'Std']],frames:['one-shot'] });
    expect(handle.setCrewReadiness).toHaveBeenCalledWith(
      { connected: 3, ready: 3 }, [['helm', 'Std']]);
    options.onWelcome();
    await Promise.resolve();
    expect(sent).toContainEqual({ kind: 'fleet_join_status', status: 'admitted' });
    expect(handle.update).toHaveBeenCalledTimes(2);
    expect(handle.broadcast).toHaveBeenCalledExactlyOnceWith('one-shot');
  });
  it('opens one relay-only owner carrying ship and GM capabilities', async () => {
    const sent = [];
    let options;
    const handle = {
      role: 'ship-gm', update: vi.fn(), setCrewReadiness: vi.fn(),
      setGmReady: vi.fn(), setStartValidation: vi.fn(), broadcast: vi.fn(), close: vi.fn(),
    };
    const peer = createNativeFleetPeer({
      send: record => sent.push(record),
      createOwner: candidate => { options = candidate; return handle; },
    });
    expect(peer.configure({
      base: 'https://fleet.test',
      stamp: '14/base/3', stamp_valid:true, max_slots: 4,
      max_name_length: 48, max_ship_path_length: 160,
      ship_path: 'assets/entities/ship.toml', gm_name: 'GM', operator_id: 'native-gm',
      credentials: ['owner-secret', 'join-secret'],
    })).toBe(true);
    expect(options.role).toBe('ship-gm');
    expect(options.transports).toEqual(['ws-relay']);
    expect(options.ship.template_path).toBe('assets/entities/ship.toml');
    expect(options.ownerOperatorId).toBe('native-gm');
    expect(options.credentialFactory()).toBe('owner-secret');
    expect(options.checkStamp('14/base/3')).toEqual({ok:true});
    for (const stamp of [null,'','14//3','15/base/3','14/base/4','14/other/3','{"protocol":14,"content_id":"base","content_epoch":3}']) {
      expect(options.checkStamp(stamp).ok).toBe(false);
    }

    options.onCode({ full: 'server_code', suffix: 'CODE' });
    const adoption = options.onSimulationRoster({ frozen: true, local: 1, participants: [1, 2] });
    options.onSimulationFrame('{"m":14}', 2);
    expect(sent.map(record => record.kind)).toEqual([
      'fleet_code', 'fleet_roster', 'fleet_frame',
    ]);
    peer.update({ roster_result: { generation: 1, accepted: true }, frames: ['first'] });
    await expect(adoption).resolves.toBe(true);
    expect(handle.broadcast).toHaveBeenCalledWith('first');
  });

  it('proxies the fleet WebSocket through the native bridge', async () => {
    const sent = [];
    let options;
    const peer = createNativeFleetPeer({
      send: record => sent.push(record),
      createOwner: candidate => {
        options = candidate;
        return {
          role: 'ship-gm', update() {}, setCrewReadiness() {}, setGmReady() {},
          setStartValidation() {}, broadcast() {}, close() {},
        };
      },
    });
    peer.configure({ base: 'https://fleet.test', max_slots: 4, credentials: ['one'] });
    const socket = options.factories.socket();
    const received = vi.fn();
    socket.onmessage = received;
    socket.send('{"type":"host-open"}');
    peer.receive('{"type":"ready"}');
    await Promise.resolve();
    expect(sent).toContainEqual({ kind: 'fleet_wire_send', generation: 0, frame: '{"type":"host-open"}' });
    expect(received).toHaveBeenCalledWith({ data: '{"type":"ready"}' });
  });

  it('feeds one native simulation handle without creating another peer', () => {
    const handle = {
      role: 'ship-gm', update: vi.fn(), setCrewReadiness: vi.fn(),
      setGmReady: vi.fn(), setStartValidation: vi.fn(), broadcast: vi.fn(), close: vi.fn(),
    };
    const createOwner = vi.fn(() => handle);
    const peer = createNativeFleetPeer({ send: vi.fn(), createOwner });
    peer.configure({ base: 'https://fleet.test', max_slots: 4, credentials: ['one'] });
    peer.update({
      ship: { template_path: 'ship.toml' }, ship_ready: true,
      crew: { connected: 2, ready: 2 }, station_ratings: [['helm', 'Std']],
      gm_ready: true, validation: true, frames: ['one', 'two'],
    });
    expect(createOwner).toHaveBeenCalledTimes(1);
    expect(handle.update).toHaveBeenCalledWith({
      ship: { template_path: 'ship.toml' }, ready: true,
    });
    expect(handle.setGmReady).toHaveBeenCalledWith(true);
    expect(handle.broadcast.mock.calls.map(call => call[0])).toEqual(['one', 'two']);
  });

  it('joins as a relay-only GM and restores its private reconnect capability', async () => {
    const sent = [];
    let options;
    const handle = {
      role: 'gm', slot: 'slot-2', update: vi.fn(), setCrewReadiness: vi.fn(),
      setGmReady: vi.fn(), setStartValidation: vi.fn(), broadcast: vi.fn(),
      forceStart: vi.fn(), close: vi.fn(),
    };
    const createMember = vi.fn(candidate => { options = candidate; return handle; });
    const peer = createNativeFleetPeer({
      send: record => sent.push(record), createMember, createOwner: vi.fn(),
    });
    expect(peer.configure({
      base: 'https://fleet.test', owner: false, stamp: '{"protocol":14}',
      credentials: ['configuration-proof'], gm_name: 'Morgan',
    })).toBe(true);
    expect(peer.join('SERVER-ABCD', { namespaces: {} }, {
      reconnectCredential: 'private-capability', claim: 'slot-2',
    })).toBe(true);
    expect(options).toMatchObject({
      code: 'SERVER-ABCD', role: 'gm', name: 'Morgan',
      reconnectCredential: 'private-capability', claim: 'slot-2',
      transports: ['ws-relay'], levers:{mode:'ws-relay',wsRelay:'only',pinned:true},
    });

    options.onIdentity({
      role: 'gm', operatorId: 'gm-1', reconnectCredential: 'private-capability',
      rolePreset: null,
    });
    await Promise.resolve();
    expect(sent).toContainEqual({
      kind: 'fleet_identity',
      identity: {
        role: 'gm', operatorId: 'gm-1', reconnectCredential: 'private-capability',
        rolePreset: null, claim: 'slot-2',
      },
    });

    const bootstrap = options.onGmJoinBootstrap(7, { frozen: true, local: 2 });
    expect(sent.at(-1)).toMatchObject({ kind: 'fleet_gm_bootstrap', id: 7, generation: 1 });
    peer.update({ roster_result: { generation: 1, accepted: true }, force_start: true });
    await expect(bootstrap).resolves.toBe(true);
    expect(handle.forceStart).toHaveBeenCalledOnce();
  });
});


describe('native continuation bridge', () => {
  function rig() {
    const sent = [];
    let options;
    const handle = { update() {}, setCrewReadiness() {}, setGmReady() {}, setStartValidation() {}, broadcast() {}, close() {} };
    const peer = createNativeFleetPeer({ send: record => sent.push(record), createMember: opts => { options = opts; return handle; } });
    peer.configure({ base: 'https://fleet.test', owner: false, credentials: ['one'], stamp: '4/base/1', stamp_valid: true });
    peer.join('CODE', {}, null, 'ship');
    return { peer, sent, options };
  }
  const wire = (peer, generation, event, frame) => peer.receive(JSON.stringify({native_wire:{generation,event,frame}}));
  it('opens a host socket beside the member and isolates delayed frames by generation', async () => {
    const { peer, sent, options } = rig();
    wire(peer, 0, 'frame', 'early');
    const joined = options.factories.socket('wss://fleet.test/v1/join');
    joined.onmessage = vi.fn();
    await Promise.resolve();
    expect(joined.onmessage).toHaveBeenCalledWith({data:'early'});
    const host = options.factories.socket('wss://fleet.test/v1/host');
    host.onmessage = vi.fn(); host.onopen = vi.fn();
    expect(host.readyState).toBe(0);
    expect(sent).toContainEqual({kind:'fleet_wire_open',generation:1,role:'host'});
    expect(() => options.factories.socket('wss://fleet.test/v1/join')).toThrow('socket limit');
    wire(peer, 1, 'open');
    host.send('takeover');
    expect(sent).toContainEqual({kind:'fleet_wire_send',generation:1,frame:'takeover'});
    joined.close();
    wire(peer, 0, 'frame', 'stale');
    wire(peer, 1, 'frame', 'hosted');
    expect(joined.onmessage).toHaveBeenCalledTimes(1);
    expect(host.onmessage).toHaveBeenCalledExactlyOnceWith({data:'hosted'});
    expect(host.onopen).toHaveBeenCalledTimes(1);
    expect(sent).toContainEqual({kind:'fleet_wire_close',generation:0});
  });
  it('refuses a role socket to a different service and reports a failed open once', async () => {
    const { peer, options } = rig();
    options.factories.socket('wss://fleet.test/v1/join');
    expect(() => options.factories.socket('wss://elsewhere.test/v1/host')).toThrow('socket endpoint');
    const host = options.factories.socket('wss://fleet.test/v1/host');
    host.onclose = vi.fn();
    wire(peer, 2, 'close'); wire(peer, 2, 'close');
    expect(host.readyState).toBe(3);
    expect(host.onclose).toHaveBeenCalledTimes(1);
  });
  it('waits for the matching completed Rust stage and retains the watermark', async () => {
    const { peer, sent, options } = rig();
    let resolved = false;
    const result = options.onContinuation({op:'replayed',epoch:1}).then(value => {resolved=true; return value;});
    expect(sent).toContainEqual({kind:'fleet_continuation',generation:1,request:{op:'replayed',epoch:1}});
    await expect(options.onContinuation({op:'commit',epoch:1})).resolves.toMatchObject({status:'refused'});
    peer.update({continuation_result:{generation:1,status:{status:'pending'}}});
    peer.update({continuation_result:{generation:1,status:{status:'idle'}}});
    peer.update({continuation_result:{generation:2,status:{status:'replayed',loss_tick:71}}});
    await Promise.resolve(); expect(resolved).toBe(false);
    peer.update({continuation_result:{generation:1,status:{status:'replayed',loss_tick:71}}});
    await expect(result).resolves.toEqual({status:'replayed',loss_tick:71});
    options.onContinuationFrame('mesh', 3, 1);
    expect(sent).toContainEqual({kind:'fleet_continuation_frame',frame:'mesh',source:3,epoch:1});
    options.onHostLost(2); options.onSlotClaimed(4);
    expect(sent).toContainEqual({kind:'fleet_host_lost',slot:2});
    expect(sent).toContainEqual({kind:'fleet_slot_claimed',slot:4});
    expect(options.checkStamp('4/base/1')).toEqual({ok:true});
    expect(options.checkStamp('')).toEqual({ok:false,code:'version-mismatch'});
  });
  it('propagates authoritative malformed replay refusal and a terminal reload fault', async () => {
    const { peer, sent, options } = rig();
    const result = options.onContinuation({op:'replayed',epoch:1});
    peer.update({continuation_result:{generation:1,status:{status:'refused',reason:'malformed-continuation-frame'}}});
    await expect(result).resolves.toEqual({status:'refused',reason:'malformed-continuation-frame'});
    options.factories.socket('wss://fleet.test/v1/join');
    wire(peer, 0, 'fault', 'native-fleet-surface-reloaded');
    expect(sent).toContainEqual({kind:'fleet_wire_adopt'});
    expect(sent).toContainEqual({kind:'fleet_fault',reason:'native-fleet-surface-reloaded',detail:''});
  });
  it('resolves outstanding continuation as refused when the fleet closes', async () => {
    const {peer,options} = rig();
    const result = options.onContinuation({op:'begin',epoch:1});
    peer.close();
    await expect(result).resolves.toEqual({status:'refused',reason:'fleet-closed'});
  });
});
