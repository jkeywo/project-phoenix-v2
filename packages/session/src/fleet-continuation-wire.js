import { CONTINUATION_WIRE, isContinuationEnvelope } from './fleet-owner-continuation.js';

const CHARS_PER_CHUNK = 12000;
const MAX_MESSAGE_CHARS = 8 * 1024 * 1024;

/** Bound each relay frame even when a retained recovery tail contains snapshots. */
export function sendContinuationWire(send, raw) {
  if (raw.length <= CHARS_PER_CHUNK) return send(raw);
  if (raw.length > MAX_MESSAGE_CHARS) throw new Error('continuation-message-overflow');
  const count = Math.ceil(raw.length / CHARS_PER_CHUNK);
  for (let index = 0; index < count; index++) send(JSON.stringify({
    continuation: CONTINUATION_WIRE, kind: 'chunk',
    body: { index, count, text: raw.slice(index * CHARS_PER_CHUNK, (index + 1) * CHARS_PER_CHUNK) },
  }));
}

/** One bounded assembly per reliable ordered authenticated connection. */
export function createContinuationWireReceiver() {
  let pending = null;
  return value => {
    if (!isContinuationEnvelope(value) || value.kind !== 'chunk') {
      if (pending) throw new Error('interleaved-continuation-message');
      return value;
    }
    const body = value.body;
    if (!body || !Number.isSafeInteger(body.index) || !Number.isSafeInteger(body.count)
        || body.count < 2 || body.count > Math.ceil(MAX_MESSAGE_CHARS / CHARS_PER_CHUNK)
        || typeof body.text !== 'string' || body.text.length > CHARS_PER_CHUNK) {
      throw new Error('invalid-continuation-chunk');
    }
    if (!pending) {
      if (body.index !== 0) throw new Error('continuation-chunk-gap');
      pending = { count: body.count, parts: [], length: 0 };
    }
    if (body.count !== pending.count || body.index !== pending.parts.length) throw new Error('continuation-chunk-gap');
    pending.parts.push(body.text); pending.length += body.text.length;
    if (pending.length > MAX_MESSAGE_CHARS) throw new Error('continuation-message-overflow');
    if (pending.parts.length !== pending.count) return null;
    const raw = pending.parts.join(''); pending = null;
    const complete = JSON.parse(raw);
    if (!isContinuationEnvelope(complete) || complete.kind === 'chunk') throw new Error('invalid-continuation-message');
    return complete;
  };
}

/** A wire owns framing and assembly for one connection generation. Repeated
 * acceptance preserves an assembly; replacement or disposal abandons it. */
export function createContinuationWire(send) {
  let generation = Symbol('initial');
  let receive = createContinuationWireReceiver();
  let closed = false;
  return {
    accept(next) {
      if (closed) return;
      if (next !== generation) { generation = next; receive = createContinuationWireReceiver(); }
    },
    receive(value) { if (closed) throw new Error('continuation-wire-closed'); return receive(value); },
    send(raw) { if (closed) throw new Error('continuation-wire-closed'); return sendContinuationWire(send, raw); },
    close() { closed = true; receive = createContinuationWireReceiver(); },
  };
}
