import {HOST_MESH_PROTOCOL} from './host-mesh.js';
export const FLEET_MEDIA_BOUNDS = {maxBytes: 20480, maxBufferedBytes: 32768};
/** Ephemeral extension of the same host envelope, never a continuation record. */
export function mediaHostFrame(raw) {
  let value;
  try {
    if (typeof raw === 'string') {
      if (raw.length > 20480 || new TextEncoder().encode(raw).byteLength > 20480) return null;
      value = JSON.parse(raw);
    } else {
      value = raw; const encoded = JSON.stringify(value);
      if (encoded.length > 20480 || new TextEncoder().encode(encoded).byteLength > 20480) return null;
    }
  } catch { return null; }
  const body = value?.d;
  if (value?.m !== HOST_MESH_PROTOCOL || value.t !== 'media' || value.tick !== null || !body
      || !Number.isSafeInteger(body.from) || body.from <= 0 || body.from > 0xffffffff
      || !Number.isSafeInteger(body.to) || body.to <= 0 || body.to > 0xffffffff || body.from === body.to
      || typeof body.call_origin_id !== 'string' || !body.call_origin_id.length || body.call_origin_id.length > 128
      || (body.generation_floor !== undefined && (!Number.isSafeInteger(body.generation_floor) || body.generation_floor < 0 || body.generation_floor > 0xffffffff))
      || (body.packet != null && (body.generation_floor ?? 0) !== 0)
      || (body.generation !== null && (!Number.isSafeInteger(body.generation) || body.generation <= 0 || body.generation > 0xffffffff))) return null;
  return value;
}
/** Adapter receipt clock bounds before any ordinary fleet staging. */
export function createFleetMediaGate(now = () => performance.now()) {
  const arrivals = []; let clock = 0;
  return raw => {
    const frame = mediaHostFrame(raw); if (!frame) return null;
    const time = now(); if (!Number.isFinite(time) || time < clock) return null;
    while (arrivals.length && time - arrivals[0][0] >= 1000) arrivals.shift();
    const encoded = typeof raw === 'string' ? raw : JSON.stringify(raw), bytes = new TextEncoder().encode(encoded).byteLength;
    if (arrivals.length >= 24 || arrivals.reduce((sum, entry) => sum + entry[1], 0) + bytes > 128 * 1024) return null;
    clock = time; arrivals.push([time, bytes]); return {frame, raw: encoded};
  };
}
