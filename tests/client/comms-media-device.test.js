import {describe, it, expect, vi} from 'vitest';
import {createCommsMediaDevice} from '../../gui/comms-media-device.js';

function fixture({permission, surface, secure = true} = {}) {
  const doc = new EventTarget(); doc.hidden = false;
  const win = new EventTarget();
  Object.assign(win, {document: doc, isSecureContext: secure,
    PhoenixOperatorCapabilities: {surface}, AudioContext: function() {}, AudioWorkletNode: function() {},
    setInterval: vi.fn(() => 1), clearInterval: vi.fn(), performance: {now: () => 1000}});
  const track = new EventTarget(); track.stop = vi.fn();
  const stream = {getTracks: () => [track]};
  const devices = {getUserMedia: vi.fn(() => permission || Promise.resolve(stream))};
  const node = () => ({connect: vi.fn(), disconnect: vi.fn()});
  const capture = {...node(), port: {onmessage: null}};
  const played = [];
  const context = {state: 'running', currentTime: 1, destination: {},
    resume: vi.fn(async () => {}), close: vi.fn(async () => {}),
    audioWorklet: {addModule: vi.fn(async () => {})},
    createMediaStreamSource: vi.fn(node), createGain: () => ({...node(), gain: {value: 1}}),
    createBuffer: vi.fn((channels, length) => ({getChannelData: () => new Float32Array(length)})),
    createBufferSource: () => {const source = {...node(), start: vi.fn(), stop: vi.fn()}; played.push(source); return source;},
  };
  const states = [], onAudio = vi.fn();
  const owner = createCommsMediaDevice({win, doc, devices, createContext: () => context,
    createCapture: () => capture, onState: state => states.push(state)});
  return {owner, doc, win, track, stream, devices, context, capture, played, states, onAudio};
}
describe('local Comms device owner', () => {
  it('keeps one pending video encode and closes a late decoded frame after stop', async () => {
    const f = fixture();
    const bytes = new Uint8Array([255,216,255,192,0,11,8,0,180,1,64,1,1,17,0,255,217]);
    let interval, encode, finishBytes, finishDecode;
    const drawImage = vi.fn(), targetDraw = vi.fn(), onVideo = vi.fn();
    const video = {readyState:2,play:vi.fn(async()=>{}),pause:vi.fn()};
    const canvas = {getContext:()=>({drawImage}),toBlob:vi.fn(callback=>{encode=callback;})};
    f.doc.createElement = tag => tag === 'video' ? video : canvas;
    f.win.setInterval.mockImplementation(callback=>{interval=callback; return 1;});
    f.win.Blob = Blob;
    f.win.createImageBitmap = vi.fn(()=>new Promise(resolve=>{finishDecode=resolve;}));
    expect(await f.owner.start('audio_video',{onVideo})).toBe(true);
    interval(); interval();
    expect(canvas.toBlob).toHaveBeenCalledTimes(1);
    const encoding = encode({size:bytes.length,arrayBuffer:()=>new Promise(resolve=>{finishBytes=resolve;})});
    const decoded = f.owner.playVideo(bytes, {getContext:()=>({drawImage:targetDraw})});
    expect(await f.owner.playVideo(bytes, {})).toBe(false);
    expect(f.win.createImageBitmap).toHaveBeenCalledTimes(1);
    f.owner.stop();
    const bitmap = {width:320,height:180,close:vi.fn()};
    finishBytes(bytes.buffer); finishDecode(bitmap);
    await encoding; expect(await decoded).toBe(false);
    expect(onVideo).not.toHaveBeenCalled(); expect(targetDraw).not.toHaveBeenCalled();
    expect(bitmap.close).toHaveBeenCalledTimes(1);
    expect(f.track.stop).toHaveBeenCalledTimes(1);
    f.owner.dispose();
  });
  it('requires supported secure browser capture, with explicit native pane unavailability', async () => {
    for (const options of [{secure: false}, {surface: 'native-pane'}]) {
      const f = fixture(options);
      expect(await f.owner.start('audio')).toBe(false);
      expect(f.devices.getUserMedia).not.toHaveBeenCalled();
      expect(f.states.at(-1)).toBe('unavailable');
      f.owner.dispose();
    }
  });
  it('cleans permission denial and never resumes a request stopped while consent is pending', async () => {
    const denied = fixture({permission: Promise.reject(new Error('denied'))});
    expect(await denied.owner.start('audio')).toBe(false);
    expect(denied.states.at(-1)).toBe('unavailable');
    denied.owner.dispose();
    let allow;
    const pending = fixture({permission: new Promise(resolve => {allow = resolve;})});
    const started = pending.owner.start('audio');
    pending.owner.stop(); allow(pending.stream);
    expect(await started).toBe(false);
    expect(pending.track.stop).toHaveBeenCalledTimes(1);
    expect(pending.context.resume).not.toHaveBeenCalled();
    pending.owner.dispose();
  });
  it('uses real production block encoding and stops tracks/nodes on page hide', async () => {
    const f = fixture();
    expect(await f.owner.start('audio', {onAudio: f.onAudio})).toBe(true);
    f.capture.port.onmessage({data: {samples: new Float32Array(1280), capture_ms: 80}});
    expect(f.onAudio).toHaveBeenCalledWith(expect.any(String), 80);
    expect(f.onAudio.mock.calls[0][0].length).toBe(3416);
    f.doc.hidden = true; f.doc.dispatchEvent(new Event('visibilitychange'));
    expect(f.track.stop).toHaveBeenCalledTimes(1);
    expect(f.capture.port.onmessage).toBeNull();
    expect(f.context.close).toHaveBeenCalledTimes(1);
    expect(f.owner.playAudio(new Uint8Array(2560))).toBe(false);
    f.owner.dispose();
  });
  it('never schedules more than 200ms of audio and disposes outstanding playback', async () => {
    const f = fixture(); await f.owner.start('audio');
    expect(f.owner.playAudio(new Uint8Array(2560))).toBe(true);
    expect(f.owner.playAudio(new Uint8Array(2560))).toBe(true);
    expect(f.owner.playAudio(new Uint8Array(2560))).toBe(false);
    expect(f.context.createBuffer).toHaveBeenCalledTimes(2);
    f.owner.stop();
    expect(f.played.every(source => source.stop.mock.calls.length === 1)).toBe(true);
    f.owner.dispose();
  });
});
