import { expect, it } from 'vitest';
import { continuationEnvelope } from '../../gui/fleet-owner-continuation.js';
import { sendContinuationWire, createContinuationWireReceiver, createContinuationWire } from '../../gui/fleet-continuation-wire.js';

it('chunks a large escaped tail below the relay frame bound and reassembles exactly', () => {
  const original=continuationEnvelope('tail',{rows:[{raw:'"\\'.repeat(90000)}]});
  const sent=[];
  sendContinuationWire(raw=>sent.push(raw),original);
  expect(sent.length).toBeGreaterThan(1);
  expect(sent.every(raw=>Buffer.byteLength(JSON.stringify({payload:raw}))<262144)).toBe(true);
  const receive=createContinuationWireReceiver();
  const results=sent.map(raw=>receive(JSON.parse(raw))).filter(Boolean);
  expect(results).toEqual([JSON.parse(original)]);
});

it('refuses missing, interleaved, oversized and nested chunks', () => {
  const sent=[];sendContinuationWire(raw=>sent.push(JSON.parse(raw)),continuationEnvelope('tail',{raw:'x'.repeat(13000)}));
  expect(()=>createContinuationWireReceiver()(sent[1])).toThrow('gap');
  const receive=createContinuationWireReceiver();receive(sent[0]);
  expect(()=>receive({continuation:'phoenix-fleet-continuation-v1',kind:'stream'})).toThrow('interleaved');
  expect(()=>sendContinuationWire(()=>{},'x'.repeat(8*1024*1024+1))).toThrow('overflow');
});

it('wire replacement abandons partial assembly, while duplicate acceptance preserves it', () => {
  const sent = [], wire = createContinuationWire(raw => sent.push(JSON.parse(raw)));
  wire.send(continuationEnvelope('tail', { raw: 'x'.repeat(13000) }));
  wire.accept(1); expect(wire.receive(sent[0])).toBeNull();
  wire.accept(1); expect(wire.receive(sent[1]).kind).toBe('tail');
  wire.receive(sent[0]); wire.accept(2);
  expect(() => wire.receive(sent[1])).toThrow('gap');
  wire.close(); expect(() => wire.receive(sent[0])).toThrow('closed');
  expect(() => wire.send('{}')).toThrow('closed');
});
it('refuses a nested chunk envelope assembled from otherwise valid fragments', () => {
  const raw = continuationEnvelope('chunk', { raw: 'x'.repeat(13000) }), frames = [];
  sendContinuationWire(value => frames.push(JSON.parse(value)), raw);
  const receive = createContinuationWireReceiver();
  receive(frames[0]); expect(() => receive(frames[1])).toThrow('invalid-continuation-message');
});
