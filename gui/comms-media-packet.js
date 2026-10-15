/** Closed, independently usable media packets. No device, route or authority. */
export const MEDIA_LIMITS = Object.freeze({
  jsonBytes: 20480, audioBytes: 2560, audioSamples: 1280, audioRate: 16000,
  audioMs: 80, playbackMs: 200, jpegBytes: 12288, width: 320, height: 180,
  videoMs: 500, sendBufferBytes: 32768,
});
const fields = ['call_origin_id', 'capture_ms', 'data', 'kind', 'lease_generation', 'sequence'];
const u32 = n => Number.isInteger(n) && n >= 0 && n <= 0xffffffff;
const identity = s => typeof s === 'string' && s.trim().length > 0
  && s.length <= 128 && new TextEncoder().encode(s).byteLength <= 128 && !/\p{Cc}/u.test(s);

export function bytesToBase64(bytes) {
  if (!(bytes instanceof Uint8Array) || bytes.length > MEDIA_LIMITS.jpegBytes) return null;
  let binary = '';
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
}

/** Checks bounded frame dimensions before invoking any browser image decoder. */
export function jpegDimensions(bytes) {
  if (!(bytes instanceof Uint8Array) || bytes.length < 12 || bytes.length > MEDIA_LIMITS.jpegBytes
      || bytes[0] !== 0xff || bytes[1] !== 0xd8
      || bytes.at(-2) !== 0xff || bytes.at(-1) !== 0xd9) return null;
  let offset = 2;
  let dimensions = null;
  while (offset + 4 <= bytes.length) {
    if (bytes[offset++] !== 0xff) return null;
    while (bytes[offset] === 0xff) offset++;
    const marker = bytes[offset++];
    if (marker === 0xd9) return offset === bytes.length ? dimensions : null;
    if (marker === undefined) return null;
    const length = (bytes[offset] << 8) | bytes[offset + 1];
    if (length < 2 || offset + length > bytes.length) return null;
    if ((marker >= 0xc0 && marker <= 0xcf) && ![0xc4, 0xc8, 0xcc].includes(marker)) {
      // One baseline still frame, with a bounded 8-bit surface. No later
      // redefinition or DNL may change the decoder's allocation after this.
      if (marker !== 0xc0 || dimensions || length < 8 || bytes[offset + 2] !== 8) return null;
      const height = (bytes[offset + 3] << 8) | bytes[offset + 4];
      const width = (bytes[offset + 5] << 8) | bytes[offset + 6];
      if (!(width > 0 && height > 0 && width <= MEDIA_LIMITS.width && height <= MEDIA_LIMITS.height
        && width * height <= 57600)) return null;
      dimensions = {width, height};
    }
    if (marker === 0xdc) return null;
    if (marker === 0xda) {
      if (!dimensions) return null;
      offset += length;
      // Baseline entropy data permits stuffed FF and restart markers only.
      while (offset < bytes.length - 2) {
        if (bytes[offset++] !== 0xff) continue;
        const next = bytes[offset++];
        if (next !== 0 && !(next >= 0xd0 && next <= 0xd7)) return null;
      }
      return offset === bytes.length - 2 ? dimensions : null;
    }
    offset += length;
  }
  return offset === bytes.length - 2 ? dimensions : null;
}

export function validateMediaPacket(value) {
  if (!value || typeof value !== 'object' || Array.isArray(value)
      || Object.keys(value).sort().join() !== fields.join()
      || !identity(value.call_origin_id) || !u32(value.lease_generation) || value.lease_generation === 0
      || !u32(value.sequence) || value.sequence === 0 || !u32(value.capture_ms)
      || !['pcm16_mono_16k', 'jpeg'].includes(value.kind) || typeof value.data !== 'string'
      || value.data.length > 16384 || value.data.length % 4 !== 0
      || !/^[A-Za-z0-9+/]*={0,2}$/.test(value.data)) return null;
  let decoded;
  try {decoded = atob(value.data);} catch {return null;}
  if (btoa(decoded) !== value.data) return null;
  const bytes = Uint8Array.from(decoded, c => c.charCodeAt(0));
  let dimensions = null;
  if (value.kind === 'pcm16_mono_16k') {
    if (bytes.length !== MEDIA_LIMITS.audioBytes) return null;
  } else {
    dimensions = jpegDimensions(bytes);
    if (!dimensions) return null;
  }
  return {packet: {...value}, bytes, dimensions};
}

export function encodeMediaPacket(packet) {
  if (!validateMediaPacket(packet)) return null;
  const encoded = JSON.stringify({type: 'CommsMediaFrame', data: packet});
  return new TextEncoder().encode(encoded).byteLength <= MEDIA_LIMITS.jsonBytes ? encoded : null;
}

export function encodePcm(samples) {
  if (!(samples instanceof Float32Array) || samples.length !== MEDIA_LIMITS.audioSamples
      || !samples.every(Number.isFinite)) return null;
  const bytes = new Uint8Array(MEDIA_LIMITS.audioBytes);
  const view = new DataView(bytes.buffer);
  samples.forEach((value, index) => {
    const sample = Math.max(-1, Math.min(1, value));
    view.setInt16(index * 2, Math.round(sample * (sample < 0 ? 32768 : 32767)), true);
  });
  return bytesToBase64(bytes);
}

/** Actual monotonic receiver clock, never a remote capture timestamp or game clock. */
export function createMediaRateGate(now = () => performance.now()) {
  const arrivals = [];
  let lastVideo = -Infinity;
  return {
    accept(kind) {
      const time = now();
      if (!Number.isFinite(time)) return false;
      while (arrivals.length && arrivals[0] <= time - 1000) arrivals.shift();
      if (arrivals.length >= 16 || (kind === 'jpeg' && time - lastVideo < MEDIA_LIMITS.videoMs)) return false;
      arrivals.push(time);
      if (kind === 'jpeg') lastVideo = time;
      return true;
    },
    reset() {arrivals.length = 0; lastVideo = -Infinity;},
  };
}
