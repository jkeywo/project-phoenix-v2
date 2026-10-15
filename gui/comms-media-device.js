import {MEDIA_LIMITS, jpegDimensions, encodePcm} from './comms-media-packet.js';

/** One page-local capture/playback owner. It neither selects calls nor grants authority. */
export function createCommsMediaDevice({win = window, doc = win.document,
  devices = win.navigator?.mediaDevices,
  createContext = () => new (win.AudioContext || win.webkitAudioContext)({sampleRate: 16000}),
  createCapture = context => new win.AudioWorkletNode(context, 'phoenix-comms-capture'),
  now = () => win.performance.now(), onState = () => {},
} = {}) {
  let generation = 0, stream = null, context = null, input = null, capture = null, silence = null;
  let video = null, canvas = null, timer = null, encoding = false, decoding = false;
  let began = 0, scheduledUntil = 0, playback = new Set();
  const available = () => win.isSecureContext === true && win.PhoenixOperatorCapabilities?.surface !== 'native-pane'
    && typeof devices?.getUserMedia === 'function' && typeof win.AudioWorkletNode === 'function'
    && typeof (win.AudioContext || win.webkitAudioContext) === 'function';
  function stop() {
    generation++;
    if (timer !== null) win.clearInterval(timer);
    timer = null;
    for (const track of stream?.getTracks?.() || []) track.stop();
    stream = null;
    for (const node of [input, capture, silence]) {try {node?.disconnect();} catch { /* already closed */ }}
    if (capture) capture.port.onmessage = null;
    for (const source of playback) {try {source.stop(); source.disconnect();} catch { /* completed */ }}
    playback.clear(); scheduledUntil = 0;
    if (video) {video.pause?.(); video.srcObject = null;}
    video = canvas = input = capture = silence = null;
    const old = context; context = null;
    try {void old?.close()?.catch(() => {});} catch { /* unsupported/closed */ }
    encoding = decoding = false;
    onState('stopped');
  }
  async function start(mode, {onAudio = () => {}, onVideo = () => {}} = {}) {
    stop();
    if (!available() || !['audio', 'audio_video'].includes(mode)) {onState('unavailable'); return false;}
    const epoch = generation;
    let obtained = null, localContext = null;
    try {
      obtained = await devices.getUserMedia({audio: {channelCount: 1, echoCancellation: true},
        video: mode === 'audio_video' ? {width: {ideal: 320}, height: {ideal: 180}, frameRate: {ideal: 2, max: 2}} : false});
      if (epoch !== generation) {obtained.getTracks().forEach(track => track.stop()); return false;}
      stream = obtained;
      localContext = createContext(); context = localContext;
      await localContext.resume();
      await localContext.audioWorklet.addModule(new URL('./comms-media-worklet.js', import.meta.url));
      if (epoch !== generation) return false;
      input = localContext.createMediaStreamSource(stream);
      capture = createCapture(localContext); silence = localContext.createGain(); silence.gain.value = 0;
      input.connect(capture); capture.connect(silence); silence.connect(localContext.destination);
      began = now();
      capture.port.onmessage = event => {
        if (epoch !== generation) return;
        const data = encodePcm(event.data?.samples);
        if (data !== null && Number.isSafeInteger(event.data.capture_ms)
            && event.data.capture_ms >= 0) onAudio(data, event.data.capture_ms);
      };
      for (const track of stream.getTracks()) track.addEventListener?.('ended', () => {
        if (epoch === generation) stop();
      }, {once: true});
      if (mode === 'audio_video') {
        if (typeof win.createImageBitmap !== 'function') throw new Error('unsupported video decoding');
        video = doc.createElement('video'); video.muted = true; video.playsInline = true; video.srcObject = stream;
        canvas = doc.createElement('canvas'); canvas.width = 320; canvas.height = 180;
        await video.play();
        if (epoch !== generation) return false;
        timer = win.setInterval(() => {
          if (encoding || epoch !== generation || video.readyState < 2) return;
          const offset = Math.floor(now() - began);
          encoding = true;
          try {
            canvas.getContext('2d').drawImage(video, 0, 0, 320, 180);
            canvas.toBlob(async blob => {
              try {
                if (!blob || blob.size > MEDIA_LIMITS.jpegBytes || epoch !== generation) return;
                const bytes = new Uint8Array(await blob.arrayBuffer());
                if (epoch === generation && jpegDimensions(bytes)) onVideo(bytes, offset);
              } finally {if (epoch === generation) encoding = false;}
            }, 'image/jpeg', 0.5);
          } catch {encoding = false;}
        }, MEDIA_LIMITS.videoMs);
      }
      onState('active'); return true;
    } catch {
      if (epoch === generation) {stop(); onState('unavailable');}
      else {obtained?.getTracks().forEach(track => track.stop()); try {await localContext?.close();} catch { /* closed */ }}
      return false;
    }
  }
  function playAudio(bytes) {
    if (!context || context.state !== 'running' || !(bytes instanceof Uint8Array)
        || bytes.length !== MEDIA_LIMITS.audioBytes) return false;
    const time = context.currentTime;
    const start = Math.max(time, scheduledUntil);
    if (start + MEDIA_LIMITS.audioMs / 1000 - time > MEDIA_LIMITS.playbackMs / 1000) return false;
    const buffer = context.createBuffer(1, MEDIA_LIMITS.audioSamples, MEDIA_LIMITS.audioRate);
    const samples = buffer.getChannelData(0), view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    samples.forEach((_, index) => {samples[index] = view.getInt16(index * 2, true) / 32768;});
    const source = context.createBufferSource(); source.buffer = buffer; source.connect(context.destination);
    source.onended = () => {playback.delete(source); source.disconnect();};
    playback.add(source); source.start(start); scheduledUntil = start + MEDIA_LIMITS.audioMs / 1000;
    return true;
  }
  async function playVideo(bytes, target) {
    const dimensions = jpegDimensions(bytes);
    if (!context || decoding || !dimensions || !target || typeof win.createImageBitmap !== 'function') return false;
    decoding = true; const epoch = generation;
    let bitmap;
    try {
      bitmap = await win.createImageBitmap(new win.Blob([bytes], {type: 'image/jpeg'}));
      if (epoch !== generation || bitmap.width !== dimensions.width || bitmap.height !== dimensions.height) return false;
      target.width = dimensions.width; target.height = dimensions.height;
      target.getContext('2d').drawImage(bitmap, 0, 0); return true;
    } catch {return false;} finally {bitmap?.close(); if (epoch === generation) decoding = false;}
  }
  const hide = () => {if (doc.hidden) stop();};
  doc.addEventListener('visibilitychange', hide);
  win.addEventListener('pagehide', stop);
  return {available, start, stop, playAudio, playVideo,
    dispose() {stop(); doc.removeEventListener('visibilitychange', hide); win.removeEventListener('pagehide', stop);},
  };
}
