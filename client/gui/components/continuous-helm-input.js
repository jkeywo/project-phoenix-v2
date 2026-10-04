// Shared scheduling and listener lifetime for the two continuous Helm controls.
export function createContinuousHelmInput(adapter) {
  let pointerId = null, keys = {}, target = null;
  let paintFrame = null, heartbeatFrame = null, inputFrame = null;
  let lastHeartbeat = 0, lastKeyboard = 0;
  const cancelPaint = () => {
    if (paintFrame) cancelAnimationFrame(paintFrame);
    paintFrame = null;
  };
  const cancelHeartbeat = () => {
    if (heartbeatFrame) cancelAnimationFrame(heartbeatFrame);
    heartbeatFrame = null;
  };
  const schedulePaint = () => {
    if (paintFrame) return;
    paintFrame = requestAnimationFrame(() => {
      paintFrame = null;
      adapter.paint();
    });
  };
  const samplePointer = (event) => {
    adapter.pointer(event);
    schedulePaint();
    if (adapter.immediatePointer) adapter.send();
  };
  const heartbeat = () => {
    heartbeatFrame = null;
    if (pointerId === null) return;
    const now = performance.now();
    if (now - lastHeartbeat >= 100) {
      adapter.send();
      lastHeartbeat = now;
    }
    heartbeatFrame = requestAnimationFrame(heartbeat);
  };
  const down = (event) => {
    if (adapter.auto() || pointerId !== null) return;
    pointerId = event.pointerId;
    if (target.setPointerCapture) target.setPointerCapture(pointerId);
    cancelPaint();
    if (!heartbeatFrame) heartbeatFrame = requestAnimationFrame(heartbeat);
    samplePointer(event);
    event.preventDefault();
  };
  const move = (event) => {
    if (event.pointerId === pointerId) samplePointer(event);
  };
  const up = (event) => {
    if (event.pointerId !== pointerId) return;
    pointerId = null;
    try { if (target.releasePointerCapture) target.releasePointerCapture(event.pointerId); } catch (_) { }
    cancelHeartbeat();
    cancelPaint();
    adapter.reset();
    adapter.paint();
    adapter.send();
  };
  const input = () => {
    inputFrame = null;
    const auto = adapter.auto();
    const keepPolling = Object.keys(keys).length > 0;
    if (auto || pointerId !== null) {
      if (!auto && keepPolling) inputFrame = requestAnimationFrame(input);
      return;
    }
    if (adapter.keyboard(keys)) {
      adapter.paint();
      const now = Date.now();
      if (now - lastKeyboard >= 100) { adapter.send(); lastKeyboard = now; }
      inputFrame = requestAnimationFrame(input);
    } else {
      if (adapter.hasValue()) {
        adapter.reset();
        adapter.paint();
        adapter.send();
      }
      if (keepPolling) inputFrame = requestAnimationFrame(input);
    }
  };
  const startInput = () => {
    if (!inputFrame) inputFrame = requestAnimationFrame(input);
  };
  const keydown = (event) => {
    const tag = event.target && event.target.tagName;
    if (tag === 'INPUT' || tag === 'TEXTAREA' || !adapter.keys.includes(event.code)) return;
    event.preventDefault();
    keys[event.code] = true;
    startInput();
  };
  const keyup = (event) => {
    delete keys[event.code];
    startInput();
  };
  const blur = () => {
    keys = {};
    startInput();
  };
  const pointerListeners = [['pointerdown', down], ['pointermove', move],
    ['pointerup', up], ['pointercancel', up], ['lostpointercapture', up]];
  function disconnect() {
    if (target) {
      for (const [name, listener] of pointerListeners) target.removeEventListener(name, listener);
      document.removeEventListener('keydown', keydown);
      document.removeEventListener('keyup', keyup);
      window.removeEventListener('blur', blur);
      target = null;
    }
    if (inputFrame) cancelAnimationFrame(inputFrame);
    inputFrame = null;
    cancelHeartbeat();
    cancelPaint();
  }
  return {
    hasPointer: () => pointerId !== null,
    connect(element) {
      if (target === element) return;
      disconnect();
      target = element;
      for (const [name, listener] of pointerListeners) target.addEventListener(name, listener);
      document.addEventListener('keydown', keydown);
      document.addEventListener('keyup', keyup);
      window.addEventListener('blur', blur);
    },
    disconnect,
  };
}
