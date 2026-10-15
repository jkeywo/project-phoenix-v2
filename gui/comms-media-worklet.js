/** Local microphone sampling only. No transport or game state in this realm. */
class CommsMediaCapture extends AudioWorkletProcessor {
  constructor() {super(); this.samples = new Float32Array(1280); this.used = 0; this.phase = 0; this.captureMs = 0;}
  process(inputs) {
    const mono = inputs[0]?.[0];
    if (!mono) return true;
    for (const sample of mono) {
      this.phase += 16000;
      while (this.phase >= sampleRate) {
        this.phase -= sampleRate;
        this.samples[this.used++] = sample;
        if (this.used === this.samples.length) {
          this.captureMs += 80;
          this.port.postMessage({samples: this.samples, capture_ms: this.captureMs});
          this.samples = new Float32Array(1280); this.used = 0;
        }
      }
    }
    return true;
  }
}
registerProcessor('phoenix-comms-capture', CommsMediaCapture);
