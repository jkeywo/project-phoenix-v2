import { describe, expect, it, vi } from 'vitest';
import { createNativeFleetPeer } from '../../gui/native-fleet-peer.js';

describe('native technical fleet peer', () => {
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
      stamp: '{"protocol":14,"content_id":"base","content_epoch":3}', max_slots: 4,
      max_name_length: 48, max_ship_path_length: 160,
      ship_path: 'assets/entities/ship.toml', gm_name: 'GM', operator_id: 'native-gm',
      credentials: ['owner-secret', 'join-secret'],
    })).toBe(true);
    expect(options.role).toBe('ship-gm');
    expect(options.transports).toEqual(['ws-relay']);
    expect(options.ship.template_path).toBe('assets/entities/ship.toml');
    expect(options.ownerOperatorId).toBe('native-gm');
    expect(options.credentialFactory()).toBe('owner-secret');
    expect(options.checkStamp('{"content_epoch":3,"content_id":"base","protocol":14}'))
      .toEqual({ ok: true });

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
    expect(sent).toContainEqual({ kind: 'fleet_wire_send', frame: '{"type":"host-open"}' });
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
});
