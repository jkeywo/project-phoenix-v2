// A reliable private action queue. Overflow faults the surface as one batch;
// no later request may overtake a request whose delivery was lost.
(() => {
  let records = [], bytes = 0, failed = false;
  const encoder = new TextEncoder();
  window.phoenixNativeGmOut = {
    send(record) {
      if (failed) return false;
      const size = encoder.encode(record).length;
      if (records.length >= 256 || size > 128 * 1024 || bytes + size > 512 * 1024) {
        failed = true;
        records = [JSON.stringify({kind: 'surface-fault'})];
        bytes = 0;
        return false;
      }
      records.push(record);
      bytes += size;
      return true;
    },
  };
  window.__phoenixNativeGmOutDrain = () => {
    const batch = records;
    records = [];
    bytes = 0;
    // UltralightPaneSurface uses vellum's one-record-per-line drain framing.
    // Each record is already JSON encoded, including any nested newlines.
    return batch.join('\n');
  };
})();
