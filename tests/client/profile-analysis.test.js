import { describe, expect, it } from 'vitest';
import {
  summarizeSamples, buildNativeMatrix, framesInWindow, compilerContention, validateRun, nativeRecords, backgroundCpu,
} from '../../scripts/profile-analysis.mjs';

const manifest = () => ({
  exitCode: 0, completedObservation: true, sourceClean: true,
  sourceRevision: 'a'.repeat(40), binarySha256: 'b'.repeat(64),
  contentSha256: 'c'.repeat(64), profileSha256: 'd'.repeat(64),
  buildReceiptVerified: true, provenanceUnchanged: true, isolatedState: true, frameCaptureComplete: true,
  condition: 'one', bundleSha256: 'e'.repeat(64),
  hardware: { gpu: 'test gpu', driver: 'test driver', powerMode: 'test mode', displays: [{ width: 1920, height: 1080, dpi: 96 }] },
  stationIntent: ['helm'], warmupSeconds: 40, measureSeconds: 30,
  expectedWindows: [{ monitor: 'test-display', width: 1920, height: 1080, scale: 1 }],
});
const quiet = () => [
  { elapsedSeconds: 40, competingCompilers: [] },
  { elapsedSeconds: 70, competingCompilers: [] },
];
const visible = () => ({
  elapsedSeconds: 10,
  data: { stage: 'console-visible', station: 'helm', ready: true, afk: true,
    rating: 'Backfill', lobbyShown: false, updateConsoleInstalled: true, frameWidth: 1788, frameHeight: 1080 },
});
const valid = () => ({ manifest: manifest(), processSamples: quiet(),
  workload: Array.from({ length: 35 }, (_, i) => ({ elapsedSeconds: i + 36, assetsReady: true,
    assetCount: 10, failedGlbs: 0, windows: [{ monitor: 'test-display', width: 1920, height: 1080, scale: 1 }] })),
  diagnostics: [visible(),
  ...Array.from({ length: 32 }, (_, i) => ({ ...visible(), elapsedSeconds: i + 38 }))], frames: [16, 17] });

describe('profile evidence', () => {
  it('refuses incomplete asset loading and windows on an unintended monitor or scale', () => {
    for (const patch of [{ monitor: 'wrong-display' }, { width: 1280 }, { scale: 1.25 }]) {
      const input = valid();
      Object.assign(input.workload[15].windows[0], patch);
      expect(validateRun(input).comparable).toBe(false);
    }
    const failed = valid(); failed.workload[15].failedGlbs = 1;
    expect(validateRun(failed).comparable).toBe(false);
    const loading = valid(); for (const record of loading.workload) record.assetsReady = false;
    expect(validateRun(loading).comparable).toBe(false);
    const extra = valid(); extra.workload[15].windows.push({ monitor: 'unexpected' });
    expect(validateRun(extra).comparable).toBe(false);
  });
  it('rejects stalled or truncated workload observations even when enough early rows exist', () => {
    for (const retained of [
      time => time <= 54,
      time => time >= 47 || time < 40,
      time => time < 50 || time > 58,
    ]) {
      const input = valid();
      input.manifest.condition = 'renderer';
      input.manifest.stationIntent = [];
      input.workload = input.workload.filter(record => retained(record.elapsedSeconds));
      expect(validateRun(input).comparable).toBe(false);
    }
  });
  it('reads aligned vellum series and refuses incompatible units or missing samples', () => {
    const capture = { series: {
      'native.frame': { unit: 'millis', samples: [20, 10] },
      'native.elapsed': { unit: 'seconds', samples: [0.12, 0.13] },
      'native.fixed_ticks': { unit: 'count', samples: [3, 1] },
    } };
    expect(nativeRecords({ capture }).map(r => [r.frameMs, r.fixedTicks])).toEqual([[20, 3], [10, 1]]);
    capture.series['native.frame'].unit = 'seconds';
    expect(() => nativeRecords({ capture })).toThrow('Invalid native metric');
    capture.series['native.frame'].unit = 'millis';
    capture.series['native.elapsed'].samples.pop();
    expect(() => nativeRecords({ capture })).toThrow('not aligned');
  });
  it('excludes the measured child and sampler from background CPU', () => {
    const processes = seconds => [1, 2, 3].map(id => ({ id, startedUtc: 'same', cpuSeconds: seconds * id }));
    expect(backgroundCpu([
      { elapsedSeconds: 0, processes: processes(0) },
      { elapsedSeconds: 2, processes: processes(2) },
    ], [1, 2])).toBe(3);
  });
  it('refuses a console which disappears after warm-up', () => {
    const input = valid();
    input.diagnostics[15].data.stage = 'console-not-confirmed';
    expect(validateRun(input).comparable).toBe(false);
  });
  it('uses raw samples for run percentiles and retains spikes', () => {
    const summary = summarizeSamples([...Array(99).fill(1), 100]);
    expect(summary).toMatchObject({ count: 100, mean: 1.99, p95: 1, p99: 1, max: 100, over16_67: 1 });
    expect(summarizeSamples([])).toBeNull();
    expect(() => summarizeSamples([NaN])).toThrow();
  });
  it('excludes intervals crossing warm-up and the observation end', () => {
    expect(framesInWindow([
      { type: 'frame', elapsedSeconds: 40.01, frameMs: 20 },
      { type: 'frame', elapsedSeconds: 40.02, frameMs: 10 },
      { type: 'frame', elapsedSeconds: 70.01, frameMs: 10 },
      { type: 'pane', elapsedSeconds: 50, frameMs: 900 },
    ], 40, 30)).toEqual([10]);
  });
  it('brackets every repetition and rotates all four conditions', () => {
    const matrix = buildNativeMatrix(3);
    expect(matrix).toHaveLength(36);
    for (const world of ['combat_test', 'falling_skyway']) {
      for (let repetition = 0; repetition < 3; repetition++) {
        const group = matrix.filter(t => t.world === world && t.repetition === repetition);
        expect(group[0]).toMatchObject({ condition: 'renderer', control: 'before' });
        expect(group.at(-1)).toMatchObject({ condition: 'renderer', control: 'after' });
        expect(group.filter(t => !t.control).map(t => t.condition).sort()).toEqual(['chrome', 'one', 'renderer', 'two']);
      }
    }
    expect(matrix.filter(t => !t.control && t.repetition === 0)[0].condition)
      .not.toBe(matrix.filter(t => !t.control && t.repetition === 1)[0].condition);
  });
  it('does not count a reused process id as continuous compiler CPU', () => {
    expect(compilerContention([
      { elapsedSeconds: 0, competingCompilers: [{ id: 1, startedUtc: 'old', cpuSeconds: 10 }] },
      { elapsedSeconds: 1, competingCompilers: [{ id: 1, startedUtc: 'new', cpuSeconds: 100 }] },
    ])).toEqual({ observed: true, cpuCoreEquivalentsLowerBound: 0 });
  });
  it('accepts complete evidence but refuses an unready, late or empty console', () => {
    expect(validateRun(valid()).comparable).toBe(true);
    for (const patch of [{ ready: false }, { afk: false }, { frameWidth: 0 }, { rating: 'Human' }]) {
      const input = valid();
      for (const diagnostic of input.diagnostics) Object.assign(diagnostic.data, patch);
      expect(validateRun(input).comparable).toBe(false);
    }
    const late = valid(); for (const diagnostic of late.diagnostics) diagnostic.elapsedSeconds += 40;
    expect(validateRun(late).comparable).toBe(false);
  });
  it('retains diagnostic results while rejecting contaminated or unproven comparisons', () => {
    const input = valid();
    input.processSamples[0].competingCompilers.push({ id: 2, startedUtc: 'now', cpuSeconds: 0 });
    expect(validateRun(input).reasons).toContain('Concurrent compiler/build process observed');
    for (const field of ['buildReceiptVerified', 'sourceClean', 'frameCaptureComplete', 'isolatedState']) {
      const unproven = valid(); unproven.manifest[field] = false;
      expect(validateRun(unproven).comparable).toBe(false);
    }
    const noFrames = valid(); noFrames.frames = [];
    expect(validateRun(noFrames).comparable).toBe(false);
    const crash = valid(); crash.stderr = 'thread main panicked at app.rs';
    expect(validateRun(crash).comparable).toBe(false);
  });
  it('does not dilute observation CPU with a quiet warm-up', () => {
    const input = valid();
    input.manifest.maxBackgroundCpuCores = 1;
    input.processSamples = [0, 40, 50, 60, 70].map(elapsedSeconds => ({ elapsedSeconds, competingCompilers: [],
      processes: [{ id: 99, startedUtc: 'same', cpuSeconds: Math.max(0, elapsedSeconds - 40) * 2 }] }));
    expect(validateRun(input).backgroundCpuCores).toBe(2);
    expect(validateRun(input).comparable).toBe(false);
  });
});
