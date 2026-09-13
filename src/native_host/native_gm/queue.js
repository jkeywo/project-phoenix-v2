// A reliable private action queue. Overflow faults the surface as one batch;
// no later request may overtake a request whose delivery was lost.
(() => {
  let records = [], bytes = 0, failed = false;
  let operatorRecord = null;
  const encoder = new TextEncoder();
  // A personal-profile write is not a GM action. Keep only the newest bounded
  // preference operation, outside the action queue's failure/ordering contract.
  window.phoenixNativeGmOperatorOut = {
    send(record) {
      if (encoder.encode(record).length > 1024 * 1024) return false;
      let value; try { value = JSON.parse(record); } catch (_) { return false; }
      if (value.type !== 'NativeOperator' || !['load','save'].includes(value.operation)) return false;
      operatorRecord = record; return true;
    },
  };
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
    if (operatorRecord) { batch.push(operatorRecord); operatorRecord = null; }
    const audio = window.__phoenixPrivateAudioDrain?.();
    if (audio) batch.push(audio);
    return batch.join('\n');
  };
})();
