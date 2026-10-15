import {describe, it, expect, vi} from 'vitest';
import {createCommsMediaClient} from '../../gui/comms-media-client.js';
import {encodePcm} from '../../gui/comms-media-packet.js';

function fixture() {
  let callbacks, deviceState;
  const link = {send: vi.fn(), sendSnapshotOnly: vi.fn(() => true)};
  const device = {available: () => true, start: vi.fn(async (mode, value) => {callbacks = value; return true;}),
    stop: vi.fn(), dispose: vi.fn(), playAudio: vi.fn(() => true), playVideo: vi.fn(async () => true)};
  let time = 0;
  const client = createCommsMediaClient({win: {}, getLink: () => link, now: () => time,
    createDevice: options => {deviceState = options.onState; return device;}});
  const view = {call_origin_id: 'actual-call', mode: 'audio', eligible: true, lease_generation: null, peer_lease_generation: null};
  client.setConnected(true); client.update(view);
  const packet = (generation = 2, sequence = 1) => ({call_origin_id: 'actual-call', lease_generation: generation,
    sequence, capture_ms: sequence * 80, kind: 'pcm16_mono_16k', data: encodePcm(new Float32Array(1280))});
  return {client, device, link, view, packet, callbacks: () => callbacks, deviceState: () => deviceState,
    advance(ms) {time += ms;}};
}
describe('actual selected-call browser leaf', () => {
  it('clears old call pixels before stop, rebinding or another video call can reveal them', async () => {
    const f = fixture();
    let painted = true;
    const clearRect = vi.fn(() => {painted = false;});
    const canvas = {width:320,height:180,getContext:()=>({clearRect})};
    f.client.setCanvas(canvas); expect(painted).toBe(false);
    f.client.update({...f.view,mode:'audio_video'}); await f.client.start();
    f.client.update({...f.view,mode:'audio_video',lease_generation:1,peer_lease_generation:2});
    painted = true; f.client.stop(); expect(painted).toBe(false);
    f.client.update({...f.view,call_origin_id:'next-call',mode:'audio_video'});
    await f.client.start(); expect(painted).toBe(false);
    painted = true; f.client.setCanvas(null); expect(painted).toBe(false);
    expect(clearRect).toHaveBeenCalled(); f.client.dispose();
  });
  it('never captures from accepted history or status alone; explicit start waits for actual leases', async () => {
    const f = fixture();
    expect(f.device.start).not.toHaveBeenCalled();
    expect(await f.client.start()).toBe(true);
    expect(f.link.send).toHaveBeenCalledWith('CommsMediaStart', {call_origin_id: 'actual-call'}, 'reliable');
    f.callbacks().onAudio(f.packet().data, 80);
    expect(f.link.sendSnapshotOnly).not.toHaveBeenCalled();
    f.client.update({...f.view, lease_generation: 1, peer_lease_generation: 2});
    f.callbacks().onAudio(f.packet().data, 160);
    expect(f.link.sendSnapshotOnly).toHaveBeenCalledTimes(1);
    const packet = JSON.parse(f.link.sendSnapshotOnly.mock.calls[0][0]);
    expect(packet.data.lease_generation).toBe(1);
    expect(f.client.receive(f.packet())).toBe(true);
    expect(f.client.receive(f.packet())).toBe(false);
    expect(f.client.receive(f.packet(3, 2))).toBe(false);
    f.client.dispose();
  });
  it('does not upgrade a drop-only failure to Reliable and never resumes on reconnect', async () => {
    const f = fixture(); await f.client.start();
    f.client.update({...f.view, lease_generation: 1, peer_lease_generation: 2});
    f.link.sendSnapshotOnly.mockReturnValue(false);
    f.callbacks().onAudio(f.packet().data, 80);
    expect(f.link.send.mock.calls.map(args => args[0])).toEqual(['CommsMediaStart']);
    f.client.setConnected(false); f.client.setConnected(true);
    f.client.update({...f.view, lease_generation: 3, peer_lease_generation: 4});
    expect(f.device.start).toHaveBeenCalledTimes(1);
    expect(f.client.state().active).toBe(false);
    expect(f.link.send.mock.calls.at(-1)).toEqual(['CommsMediaStop', {lease_generation: 3}, 'reliable']);
    expect(f.client.receive(f.packet(2, 5))).toBe(false);
    f.client.dispose();
  });
  it('stops on route/call/operator-derived status loss and a delayed grant cannot restart it', async () => {
    const f = fixture(); await f.client.start();
    f.client.update({...f.view, lease_generation: 1, peer_lease_generation: 2});
    f.client.update({...f.view, eligible: false});
    expect(f.client.state().active).toBe(false);
    expect(f.device.stop).toHaveBeenCalled();
    f.client.update({...f.view, lease_generation: 9, peer_lease_generation: 10});
    expect(f.client.state().active).toBe(false);
    expect(f.link.send.mock.calls.at(-1)).toEqual(['CommsMediaStop', {lease_generation: 9}, 'reliable']);
    f.client.dispose();
  });
  it('a local device end stops the host lease instead of leaving a silent active capture', async () => {
    const f = fixture(); await f.client.start();
    f.client.update({...f.view, lease_generation: 1, peer_lease_generation: 2});
    f.deviceState()('stopped');
    expect(f.client.state().active).toBe(false);
    expect(f.link.send.mock.calls.at(-1)).toEqual(['CommsMediaStop', {lease_generation: 1}, 'reliable']);
    f.client.dispose();
  });
});
