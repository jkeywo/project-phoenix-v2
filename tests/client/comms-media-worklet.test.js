import {describe, it, expect} from 'vitest';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';

describe('production microphone sampling worklet', () => {
  it('keeps fractional resampling state across callbacks and emits only complete bounded blocks', () => {
    let Processor;
    const packets = [];
    class Worklet {constructor() {this.port = {postMessage: samples => packets.push(samples)};}}
    runInNewContext(readFileSync(new URL('../../gui/comms-media-worklet.js', import.meta.url), 'utf8'), {
      AudioWorkletProcessor: Worklet, Float32Array, sampleRate: 48000,
      registerProcessor: (name, ctor) => {expect(name).toBe('phoenix-comms-capture'); Processor = ctor;},
    });
    const processor = new Processor();
    expect(processor.process([])).toBe(true);
    for (let i = 0; i < 90; i++) processor.process([[new Float32Array(128).fill(0.25)]]);
    expect(packets).toHaveLength(3);
    expect(packets.every(({samples}) => samples.length === 1280 && samples.every(value => value === 0.25))).toBe(true);
    expect(packets.map(packet => packet.capture_ms)).toEqual([80, 160, 240]);
    expect(processor.used).toBe(0);
  });
});
