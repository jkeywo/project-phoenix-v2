import {describe, it, expect} from 'vitest';
import {MEDIA_LIMITS, bytesToBase64, jpegDimensions, validateMediaPacket, encodeMediaPacket,
  encodePcm, createMediaRateGate} from '../../gui/comms-media-packet.js';

const audio = () => ({call_origin_id: 'actual-call', lease_generation: 1, sequence: 1,
  capture_ms: 80, kind: 'pcm16_mono_16k', data: encodePcm(new Float32Array(1280))});
function jpeg(width = 320, height = 180) {
  // Bounded header probe, not a claim that this is a decodable photograph.
  return new Uint8Array([255,216,255,192,0,11,8,height >> 8,height & 255,width >> 8,width & 255,1,1,17,0,255,217]);
}
describe('closed media packet', () => {
  it('keeps PCM exact, little-endian and independently usable', () => {
    const samples = new Float32Array(1280); samples[0] = -1; samples[1] = 1;
    const packet = {...audio(), data: encodePcm(samples)};
    const result = validateMediaPacket(packet);
    expect([...result.bytes.slice(0, 4)]).toEqual([0,128,255,127]);
    expect(JSON.parse(encodeMediaPacket(packet))).toEqual({type: 'CommsMediaFrame', data: packet});
    expect(encodePcm(new Float32Array(1279))).toBeNull();
    samples[0] = NaN; expect(encodePcm(samples)).toBeNull();
  });
  it('rejects oversized decoded images before any browser decoder', () => {
    expect(jpegDimensions(jpeg())).toEqual({width: 320, height: 180});
    expect(jpegDimensions(jpeg(65535, 180))).toBeNull();
    expect(jpegDimensions(jpeg(320, 181))).toBeNull();
    expect(jpegDimensions(new Uint8Array(MEDIA_LIMITS.jpegBytes + 1))).toBeNull();
    const packet = {...audio(), kind: 'jpeg', data: bytesToBase64(jpeg())};
    expect(validateMediaPacket(packet).dimensions).toEqual({width: 320, height: 180});
    expect(validateMediaPacket({...packet, data: bytesToBase64(jpeg(321, 180))})).toBeNull();
  });
  it('closes identity, counters, encoding and exact payload shape', () => {
    for (const patch of [{sequence: 0}, {lease_generation: 0}, {capture_ms: -1}, {sequence: 2**32},
      {kind: 'html'}, {token: 'foreign'}, {data: 'abcd'}, {data: 'x'.repeat(16388)},
      {call_origin_id: '\n'}, {data: audio().data + '='}]) {
      expect(validateMediaPacket({...audio(), ...patch})).toBeNull();
    }
  });
  it('bounds actual received traffic rather than trusting capture offsets', () => {
    let time = 0; const gate = createMediaRateGate(() => time);
    expect(gate.accept('jpeg')).toBe(true);
    expect(gate.accept('jpeg')).toBe(false);
    for (let i = 0; i < 15; i++) expect(gate.accept('pcm16_mono_16k')).toBe(true);
    expect(gate.accept('pcm16_mono_16k')).toBe(false);
    time = 1000; expect(gate.accept('jpeg')).toBe(true);
    gate.reset(); expect(gate.accept('jpeg')).toBe(true);
  });
});
