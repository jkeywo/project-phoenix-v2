/** Optional opaque lossy send. Never promotes ephemeral work to Reliable. */
export function sendSnapshotOnly(channel, encoded, { maxBytes, maxBufferedBytes } = {}) {
  if (typeof encoded !== 'string' || channel?.readyState !== 'open' || channel.snapshotOnlySupported === false
      || !Number.isSafeInteger(maxBytes) || maxBytes <= 0
      || !Number.isSafeInteger(maxBufferedBytes) || maxBufferedBytes < 0) return false;
  // Reject before allocating an encoded copy, then measure UTF-8 accurately.
  if (encoded.length > maxBytes) return false;
  const bytes = new TextEncoder().encode(encoded).byteLength;
  if (bytes > maxBytes) return false;
  const wireBytes = typeof channel.sendByteLength === 'function' ? channel.sendByteLength(encoded) : bytes;
  if (!Number.isSafeInteger(wireBytes) || wireBytes < bytes || wireBytes > maxBufferedBytes) return false;
  // Some native bridges expose a bounded real staging/receipt seam instead of
  // a synchronously readable socket backlog. Unknown adapters never get this.
  if (typeof channel.trySendSnapshot === 'function') {
    try {return channel.trySendSnapshot(encoded, {maxBytes, maxBufferedBytes}) === true;} catch {return false;}
  }
  const buffered = channel.bufferedAmount;
  if (!Number.isSafeInteger(wireBytes) || wireBytes < bytes || !Number.isFinite(buffered)
      || buffered < 0 || buffered + wireBytes > maxBufferedBytes) return false;
  try { return (typeof channel.sendDropOnly === 'function' ? channel.sendDropOnly(encoded) : channel.send(encoded)) !== false; } catch { return false; }
}
