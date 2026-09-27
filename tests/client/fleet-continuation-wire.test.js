import { expect, it } from 'vitest';
import { continuationEnvelope } from '../../gui/fleet-owner-continuation.js';
import { sendContinuationWire, createContinuationWireReceiver } from '../../gui/fleet-continuation-wire.js';

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
