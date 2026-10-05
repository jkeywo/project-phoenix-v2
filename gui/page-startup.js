/* Synchronous classic-script bootstrap. Feature modules publish collaborators;
 * callers can use the installed adapters immediately, before deferred imports. */
(function (scope) {
  function install(win, { host = false } = {}) {
    if (win.pageStartup) return win.pageStartup;
    let chrome = null, audio = null, channel = null, disposed = false;
    const chromeQueue = [], audioQueue = [], channelQueue = [];
    const call = (target, queue, method, args) => {
      if (disposed) return;
      if (target) return target[method](...args);
      queue.push([method, args]);
    };
    const drain = (target, queue) => {
      while (!disposed && queue.length) { const [method, args] = queue.shift(); target[method](...args); }
    };
    function dispatch(name, payload) {
      if (disposed) return;
      // Audio handlers are installed before any early Host Channel is flushed.
      if (channel && audio) return channel(name, payload);
      channelQueue.push([name, payload]);
    }
    function drainChannel() {
      while (!disposed && channel && audio && channelQueue.length) channel(...channelQueue.shift());
    }
    const startup = {
      get chrome() { return chrome; },
      callChrome(method, ...args) { return call(chrome, chromeQueue, method, args); },
      callAudio(method, ...args) { return call(audio, audioQueue, method, args); },
      dispose() {
        if (disposed) return;
        disposed = true;
        chromeQueue.length = audioQueue.length = channelQueue.length = 0;
        audio?.setPageActive(false);
        audio?.dispose?.();
        if (chrome?.dispose) chrome.dispose(); else chrome?.releaseWakeLock();
        win.removeEventListener?.('pagehide', hide);
        win.removeEventListener?.('pageshow', show);
        if (win.pageStartup === startup) delete win.pageStartup;
      },
    };
    const hide = event => {
      if (event.persisted) audio?.setPageActive(false);
      else startup.dispose();
    };
    const show = () => audio?.setPageActive(true);
    win.pageStartup = startup;
    win.__pageChromeReady = value => {
      if (disposed) { value.dispose?.(); return; }
      chrome = value; drain(chrome, chromeQueue);
    };
    if (host) {
      win.__hostChannel = dispatch;
      win.__hostChannelReady = value => { if (!disposed) { channel = value; drainChannel(); } };
      win.__hostAudioReady = value => {
        if (disposed) { value.dispose?.(); return; }
        audio = value;
        win.__roomAudio = audio; win.__resetHostAudio = audio.resetSession;
        for (const [name, method] of Object.entries({ __audioConfig: 'audioConfig', __audioCue: 'audioCue',
          __audioLevel: 'audioLevel', __audioLifecycle: 'audioLifecycle', __audioDebug: 'debug',
          __setMasterVolume: 'setMasterVolume', __getMasterVolume: 'getMasterVolume' })) win[name] = audio[method];
        drain(audio, audioQueue); drainChannel();
      };
    }
    win.addEventListener?.('pagehide', hide);
    win.addEventListener?.('pageshow', show);
    return startup;
  }
  scope.PhoenixPageStartup = { install };
})(globalThis);
