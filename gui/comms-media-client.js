import {createCommsMediaDevice} from './comms-media-device.js';
import {MEDIA_LIMITS, bytesToBase64, encodeMediaPacket, validateMediaPacket, createMediaRateGate} from './comms-media-packet.js';

/** Page-local selected-call consumer. Only actual host state supplies eligibility
 * and lease generations; a click never assumes remote acceptance or delivery. */
export function createCommsMediaClient({getLink, win = window,
  createDevice = options => createCommsMediaDevice(options),
  now = () => win.performance.now(),
} = {}) {
  let view = {eligible: false}, connected = false, active = false, starting = false;
  let epoch = 0, sequence = 0, lastSequence = 0, captureOffsets = new Map(), canvas = null, status = 'stopped';
  const listeners = new Set(), outgoing = createMediaRateGate(now), incoming = createMediaRateGate(now);
  const emit = value => {status = value; for (const listener of listeners) listener(state());};
  const sendControl = (type, data) => {if (connected) getLink()?.send(type, data, 'reliable');};
  const device = createDevice({win, onState: value => {
    if (value === 'stopped' && active) stop();
    else if (value === 'unavailable') emit('unavailable');
  }});
  function clearCanvas(target) {
    if (!target) return;
    try {target.getContext('2d')?.clearRect(0, 0, target.width, target.height);} catch { /* unavailable surface */ }
  }
  function state() {return {...view, connected, active, starting, status, supported: device.available()};}
  function stop() {
    const generation = view.lease_generation;
    epoch++; active = starting = false; sequence = lastSequence = 0;
    captureOffsets.clear(); outgoing.reset(); incoming.reset();
    view = {...view, lease_generation: null, peer_lease_generation: null};
    device.stop();
    clearCanvas(canvas);
    if (generation) sendControl('CommsMediaStop', {lease_generation: generation});
    emit('stopped');
  }
  function update(next) {
    const changed = view.call_origin_id !== next?.call_origin_id || view.mode !== next?.mode
      || (view.lease_generation && next?.lease_generation !== view.lease_generation)
      || (view.peer_lease_generation && next?.peer_lease_generation !== view.peer_lease_generation);
    if (changed || !next?.eligible) stop();
    view = next && typeof next === 'object' ? {...next} : {eligible: false};
    if (!active && !starting && view.lease_generation) {
      // A delayed successful start response cannot reopen a locally stopped device.
      sendControl('CommsMediaStop', {lease_generation: view.lease_generation});
      view.lease_generation = view.peer_lease_generation = null;
    }
    emit(active ? (view.peer_lease_generation ? 'active' : 'waiting') : 'stopped');
  }
  function publish(kind, data, capture_ms) {
    if (!active || !connected || !view.eligible || !view.lease_generation
        || !view.peer_lease_generation || !outgoing.accept(kind) || sequence === 0xffffffff) return false;
    if (kind === 'jpeg' && view.mode !== 'audio_video') return false;
    const packet = {call_origin_id: view.call_origin_id, lease_generation: view.lease_generation,
      sequence: ++sequence, capture_ms, kind, data};
    const encoded = encodeMediaPacket(packet);
    return encoded !== null && getLink()?.sendSnapshotOnly?.(encoded,
      {maxBytes: MEDIA_LIMITS.jsonBytes, maxBufferedBytes: MEDIA_LIMITS.sendBufferBytes}) === true;
  }
  async function start() {
    if (active || starting || !connected || !view.eligible || !view.call_origin_id || !device.available()) {
      if (!device.available()) emit('unavailable');
      return false;
    }
    const current = ++epoch; starting = true; emit('starting');
    const captured = await device.start(view.mode, {
      onAudio: (data, offset) => publish('pcm16_mono_16k', data, offset),
      onVideo: (bytes, offset) => publish('jpeg', bytesToBase64(bytes), offset),
    });
    if (current !== epoch) return false;
    starting = false;
    if (!captured) {emit('unavailable'); return false;}
    active = true;
    sendControl('CommsMediaStart', {call_origin_id: view.call_origin_id});
    emit('waiting'); return true;
  }
  function receive(packet) {
    const decoded = validateMediaPacket(packet);
    if (!decoded || !active || !connected || !view.eligible
        || packet.call_origin_id !== view.call_origin_id
        || packet.lease_generation !== view.peer_lease_generation
        || packet.sequence <= lastSequence
        || (packet.kind === 'jpeg' && view.mode !== 'audio_video')
        || packet.capture_ms < (captureOffsets.get(packet.kind) ?? 0)
        || !incoming.accept(packet.kind)) return false;
    lastSequence = packet.sequence; captureOffsets.set(packet.kind, packet.capture_ms);
    if (packet.kind === 'pcm16_mono_16k') return device.playAudio(decoded.bytes);
    void device.playVideo(decoded.bytes, canvas); return true;
  }
  return {start, stop, update, receive, state,
    setCanvas(value) {clearCanvas(canvas); canvas = value; clearCanvas(canvas);},
    setConnected(value) {
      if (connected === !!value) return;
      connected = !!value;
      if (!connected) {stop(); view = {eligible: false};}
      emit('stopped');
    },
    subscribe(listener) {listeners.add(listener); listener(state()); return () => listeners.delete(listener);},
    dispose() {stop(); device.dispose(); listeners.clear();},
  };
}

/** Mounted only by the actual current Comms surface; no implicit device access. */
export function mountCommsMediaControls(root, client, t) {
  const doc = root.ownerDocument, panel = doc.createElement('section');
  const status = doc.createElement('p'), start = doc.createElement('button'), stop = doc.createElement('button');
  const canvas = doc.createElement('canvas');
  status.setAttribute('role', 'status'); status.setAttribute('aria-live', 'polite');
  start.type = stop.type = 'button';
  start.textContent = t('comms.media.start'); stop.textContent = t('comms.media.stop');
  canvas.setAttribute('aria-label', t('comms.media.remote_video'));
  canvas.width = 320; canvas.height = 180;
  panel.append(status, start, stop, canvas); root.append(panel); client.setCanvas(canvas);
  start.addEventListener('click', () => {void client.start();});
  stop.addEventListener('click', () => client.stop());
  const unsubscribe = client.subscribe(view => {
    start.textContent = t(view.mode === 'audio' ? 'comms.media.start_audio' : 'comms.media.start');
    status.textContent = t(`comms.media.${view.supported ? view.status : 'unavailable'}`);
    start.disabled = !view.connected || !view.eligible || !view.supported || view.active || view.starting;
    stop.disabled = !view.active && !view.starting;
    canvas.hidden = !view.active || view.mode !== 'audio_video';
  });
  return () => {unsubscribe(); client.stop(); client.setCanvas(null); panel.remove();};
}

if (typeof window !== 'undefined') window.PhoenixCommsMedia = {createCommsMediaClient, mountCommsMediaControls};
