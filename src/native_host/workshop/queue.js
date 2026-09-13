(function () {
  var records = [], bytes = 0, encoder = new TextEncoder();
  window.__phoenixNativeWorkshopSend = function (record) {
    if (typeof record !== 'string' || /[\r\n]/.test(record)) throw new Error('Invalid Workshop queue record');
    var size = encoder.encode(record).length;
    if (size > 33554432 || records.length >= 8 || bytes + size > 33554432) throw new Error('Workshop queue unavailable');
    records.push(record); bytes += size;
  };
  window.__phoenixNativeWorkshopDrain = function () { var result = records; records = []; bytes = 0; return result.join('\n'); };
}());
