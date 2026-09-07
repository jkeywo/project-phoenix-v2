// Review-only instrumentation. Injected only in a copied client document.
// Uses ordinary admitted participant commands; never changes production files.
(function () {
  'use strict';
  var pane = window.__phoenixPane;
  var original = window.__phoenixPaneApply;
  if (!pane || typeof original !== 'function') return;
  var station = null;
  var readySent = false;
  var afkSent = false;
  var ready = false;
  var afk = false;
  var rating = null;
  var reportedVisible = false;
  var images = [];
  function report(stage, extra) {
    var data = { stage: stage, station: station, ready: ready, afk: afk, rating: rating };
    if (extra) Object.keys(extra).forEach(function (key) { data[key] = extra[key]; });
    // One small GET per state transition, captured by the parent harness.
    // No identity token or gameplay payload is sent to this local listener.
    var image = new Image();
    images.push(image);
    image.onload = image.onerror = function () { images.splice(images.indexOf(image), 1); };
    image.src = 'http://127.0.0.1:18181/?state=' + encodeURIComponent(JSON.stringify(data));
  }
  function send(type, data) {
    if (!window.phoenixLink || typeof window.phoenixLink.send !== 'function') return false;
    window.phoenixLink.send(type, data, 'reliable');
    return true;
  }
  window.__phoenixPaneApply = function (json) {
    var result = original.apply(this, arguments);
    var message;
    try { message = JSON.parse(json); } catch (_) { return result; }
    var data = message.data || {};
    if (message.type === 'StationAssigned' && data.token === pane.token) {
      station = typeof data.station_id === 'string' ? data.station_id : (data.station_id && data.station_id.id);
      if (station) report('station-assigned');
      if (station && !readySent) {
        readySent = send('SetReady', { ready: true });
        report('ready-requested');
      }
    } else if (message.type === 'ReadyChanged' && data.token === pane.token) {
      ready = !!data.ready;
      if (ready) {
        report('ready-acknowledged');
        if (!afkSent) {
          afkSent = send('SetAfk', { afk: true });
          report('afk-requested');
        }
      }
    } else if (message.type === 'AfkChanged' && data.token === pane.token) {
      afk = !!data.afk;
      report('afk-acknowledged');
    } else if (message.type === 'RatingChanged' && data.station_id === station) {
      rating = data.rating_name;
      report('rating-changed');
    }
    return result;
  };
  report('shim-installed');
  var checks = 0;
  var interval = setInterval(function () {
    checks++;
    if (station && !readySent) readySent = send('SetReady', { ready: true });
    if (ready && !afkSent) afkSent = send('SetAfk', { afk: true });
    var section = document.querySelector('.console-section.active');
    var frame = section && section.querySelector('iframe');
    var lobby = document.getElementById('lobby-ui');
    var lobbyShown = !!(lobby && lobby.getBoundingClientRect().width && lobby.getBoundingClientRect().height && getComputedStyle(lobby).display !== 'none');
    var live = false;
    try { live = !!(frame && frame.contentWindow && typeof frame.contentWindow.__updateConsole === 'function'); } catch (_) {}
    if (ready && afk && frame && live && !lobbyShown) {
      reportedVisible = true;
      report('console-visible', { section: section.id, frame: frame.id, src: frame.getAttribute('src'), frameWidth: frame.getBoundingClientRect().width, frameHeight: frame.getBoundingClientRect().height, lobbyShown: lobbyShown, updateConsoleInstalled: live });
    } else if (checks >= 30 || reportedVisible) {
      report('console-not-confirmed', { section: section && section.id, frame: frame && frame.id, lobbyShown: lobbyShown, updateConsoleInstalled: live });
    }
  }, 1000);
})();
