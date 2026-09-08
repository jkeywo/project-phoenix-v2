/**
 * Native Station iframes share their parent pane's animation scheduler.
 *
 * Ultralight can leave native rAF pending while a document is quiet. The
 * outer pane installs a timer-backed scheduler in pane_boot.js, but window
 * overrides do not cross iframe boundaries. Install this through PhElement
 * before any component is defined/upgraded, including consoles that never
 * load bg-raf-keepalive.js. The module evaluates once per document.
 *
 * Browser documents keep their existing scheduler, including the classic
 * hidden-tab keepalive on Helm. No separate cadence is introduced here.
 */
(function () {
  if (typeof window === 'undefined') return;
  var host;
  try {
    host = window.parent;
    if (!host || host === window
        || host.PhoenixOperatorCapabilities?.surface !== 'native-pane'
        || typeof host.requestAnimationFrame !== 'function'
        || typeof host.cancelAnimationFrame !== 'function') return;
  } catch (_) {
    // An unrelated cross-origin embed retains its own browser scheduler.
    return;
  }

  var request = host.requestAnimationFrame.bind(host);
  var cancel = host.cancelAnimationFrame.bind(host);
  window.requestAnimationFrame = function (callback) {
    return request(function () {
      // Parent and child documents can have different performance origins.
      // Consumers compare this timestamp with their own performance.now().
      callback(window.performance.now());
    });
  };
  window.cancelAnimationFrame = cancel;
})();
