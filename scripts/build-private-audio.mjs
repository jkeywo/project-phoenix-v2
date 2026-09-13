// Deterministic original non-speech feedback tones. PCM mono / 48 kHz / 16 bit.
// Regeneration changes only the three named private assets; no external inputs.
import { writeFileSync } from 'node:fs';
const rate = 48000;
const tones = {
  private_refused: [[440, 0.07], [330, 0.1]],
  private_timeout: [[330, 0.07], [0, 0.07], [330, 0.07]],
  private_alert: [[660, 0.09], [880, 0.13]],
};
for (const [name, notes] of Object.entries(tones)) {
  const count = notes.reduce((sum, [, seconds]) => sum + Math.round(rate * seconds), 0);
  const wav = Buffer.alloc(44 + count * 2);
  wav.write('RIFF'); wav.writeUInt32LE(wav.length - 8, 4); wav.write('WAVEfmt ', 8);
  wav.writeUInt32LE(16, 16); wav.writeUInt16LE(1, 20); wav.writeUInt16LE(1, 22);
  wav.writeUInt32LE(rate, 24); wav.writeUInt32LE(rate * 2, 28);
  wav.writeUInt16LE(2, 32); wav.writeUInt16LE(16, 34); wav.write('data', 36);
  wav.writeUInt32LE(count * 2, 40);
  let offset = 44;
  for (const [frequency, seconds] of notes) {
    const length = Math.round(rate * seconds);
    for (let i = 0; i < length; i++) {
      const envelope = Math.min(1, i / 240, (length - i - 1) / 720);
      wav.writeInt16LE(Math.round(Math.sin(2 * Math.PI * frequency * i / rate) * envelope * 16383), offset);
      offset += 2;
    }
  }
  writeFileSync(new URL(`../assets/sounds/${name}.wav`, import.meta.url), wav);
}
