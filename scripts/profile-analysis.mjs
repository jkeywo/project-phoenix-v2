// Analysis shared by the native/headless profiling runners (#1409).
// Raw samples are authoritative. Rounded reporting-window percentiles are
// never combined into a run percentile.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export function summarizeSamples(samples) {
  if (!Array.isArray(samples) || samples.some(n => !Number.isFinite(n) || n < 0)) {
    throw new Error('Expected finite, non-negative timing samples');
  }
  if (!samples.length) return null;
  const ordered = [...samples].sort((a, b) => a - b);
  const percentile = p => ordered[Math.ceil(ordered.length * p) - 1];
  const mean = ordered.reduce((sum, value) => sum + value, 0) / ordered.length;
  return {
    count: ordered.length, mean, p50: percentile(0.5), p95: percentile(0.95),
    p99: percentile(0.99), max: ordered.at(-1),
    over16_67: ordered.filter(value => value > 1000 / 60).length,
  };
}

export function buildNativeMatrix(repetitions = 3) {
  if (!Number.isInteger(repetitions) || repetitions < 1) throw new Error('Invalid repetition count');
  const conditions = ['renderer', 'chrome', 'one', 'two'];
  const tasks = [];
  for (let repetition = 0; repetition < repetitions; repetition++) {
    const worlds = repetition % 2 ? ['falling_skyway', 'combat_test'] : ['combat_test', 'falling_skyway'];
    for (const world of worlds) {
      tasks.push({ world, condition: 'renderer', repetition, control: 'before' });
      for (let offset = 0; offset < conditions.length; offset++) {
        tasks.push({ world, condition: conditions[(offset + repetition) % conditions.length], repetition, control: null });
      }
      tasks.push({ world, condition: 'renderer', repetition, control: 'after' });
    }
  }
  return tasks;
}

export function framesInWindow(records, warmupSeconds, measureSeconds) {
  if (!(warmupSeconds >= 0 && measureSeconds > 0)) throw new Error('Invalid observation interval');
  return records.filter(record => record.type === 'frame'
    && Number.isFinite(record.elapsedSeconds) && Number.isFinite(record.frameMs)
    && record.frameMs >= 0
    && record.elapsedSeconds - record.frameMs / 1000 >= warmupSeconds
    && record.elapsedSeconds <= warmupSeconds + measureSeconds).map(record => record.frameMs);
}

export function nativeRecords(artifact) {
  const series = artifact.capture?.series || {};
  if (!artifact.capture) return [];
  for (const [metric, unit] of [['native.frame', 'millis'], ['native.elapsed', 'seconds'], ['native.fixed_ticks', 'count']]) {
    if (series[metric]?.unit !== unit || !Array.isArray(series[metric]?.samples)
      || series[metric].samples.some(n => !Number.isFinite(n) || n < 0)) throw new Error('Invalid native metric: ' + metric);
  }
  const frames = series['native.frame']?.samples || [];
  const elapsed = series['native.elapsed']?.samples || [];
  const ticks = series['native.fixed_ticks']?.samples || [];
  if (frames.length !== elapsed.length || frames.length !== ticks.length) throw new Error('Native capture series are not aligned');
  if (elapsed.some((time, i) => (i && time < elapsed[i - 1]) || time < frames[i] / 1000)) throw new Error('Invalid native elapsed clock');
  return frames.map((frameMs, i) => ({ type: 'frame', frameMs, elapsedSeconds: elapsed[i], fixedTicks: ticks[i] }));
}

export function compilerContention(samples) {
  let observed = false;
  let cpuSeconds = 0;
  for (let i = 0; i < samples.length; i++) {
    const current = samples[i].competingCompilers;
    if (!Array.isArray(current)) continue;
    observed ||= current.length > 0;
    if (!i) continue;
    const previous = new Map((samples[i - 1].competingCompilers || [])
      .map(proc => [String(proc.id) + ':' + proc.startedUtc, proc.cpuSeconds]));
    for (const proc of current) {
      const before = previous.get(String(proc.id) + ':' + proc.startedUtc);
      if (Number.isFinite(before) && Number.isFinite(proc.cpuSeconds)) {
        cpuSeconds += Math.max(0, proc.cpuSeconds - before);
      }
    }
  }
  const seconds = samples.length > 1 ? samples.at(-1).elapsedSeconds - samples[0].elapsedSeconds : 0;
  return { observed, cpuCoreEquivalentsLowerBound: seconds > 0 ? cpuSeconds / seconds : null };
}

export function backgroundCpu(samples, ignoredPids) {
  const ignored = new Set(ignoredPids);
  let cpu = 0;
  for (let i = 1; i < samples.length; i++) {
    const before = new Map((samples[i - 1].processes || []).map(p => [p.id + ':' + p.startedUtc, p.cpuSeconds]));
    for (const process of samples[i].processes || []) {
      const previous = before.get(process.id + ':' + process.startedUtc);
      if (!ignored.has(process.id) && Number.isFinite(previous) && Number.isFinite(process.cpuSeconds)) {
        cpu += Math.max(0, process.cpuSeconds - previous);
      }
    }
  }
  const elapsed = samples.length > 1 ? samples.at(-1).elapsedSeconds - samples[0].elapsedSeconds : 0;
  return elapsed > 0 ? cpu / elapsed : null;
}

export function validateRun({ manifest, processSamples = [], diagnostics = [], frames = [], workload = [], stderr = '' }) {
  const reasons = [];
  const require = (condition, reason) => { if (!condition) reasons.push(reason); };
  require(manifest.exitCode === 0, 'Host did not exit successfully');
  require(manifest.completedObservation === true, 'Observation did not finish');
  require(manifest.sourceClean === true, 'Source had uncommitted changes');
  require(typeof manifest.sourceRevision === 'string' && /^[0-9a-f]{40}$/i.test(manifest.sourceRevision),
    'Source revision is missing');
  for (const key of ['binarySha256', 'contentSha256', 'profileSha256']) {
    require(typeof manifest[key] === 'string' && /^[0-9a-f]{64}$/i.test(manifest[key]), key + ' is missing');
  }
  require(manifest.condition === 'renderer' || /^[0-9a-f]{64}$/i.test(manifest.bundleSha256 || ''), 'Bundle hash is missing');
  require(manifest.hardware?.gpu && manifest.hardware?.driver && manifest.hardware?.powerMode
    && manifest.hardware?.displays?.length > 0, 'GPU, driver, power mode or physical display provenance is missing');
  require(manifest.buildReceiptVerified === true, 'Executable has no matching build receipt');
  require(manifest.provenanceUnchanged === true, 'Source, executable or content changed during observation');
  require(manifest.isolatedState === true, 'Run did not isolate saved state');
  require(manifest.frameCaptureComplete === true, 'Raw frame capture did not close successfully');
  require(frames.length > 0, 'No raw frame samples after warm-up');
  require(Array.isArray(manifest.expectedWindows) && manifest.expectedWindows.length > 0,
    'Expected physical windows are missing');
  const workloadReady = record => record.assetsReady === true && record.assetCount > 0 && record.failedGlbs === 0
    && record.windows?.length === manifest.expectedWindows?.length
    && (manifest.expectedWindows || []).every(expected => (record.windows || []).some(window =>
      window.monitor === expected.monitor && window.width === expected.width && window.height === expected.height
        && Math.abs(window.scale - expected.scale) < 0.001));
  require(workload.some(record => record.elapsedSeconds < manifest.warmupSeconds && workloadReady(record)),
    'Loaded assets and actual physical windows were not verified before warm-up');
  const measuredWorkload = workload.filter(record => record.elapsedSeconds >= manifest.warmupSeconds
    && record.elapsedSeconds <= manifest.warmupSeconds + manifest.measureSeconds);
  require(measuredWorkload.length >= Math.floor(manifest.measureSeconds / 2)
    && measuredWorkload[0]?.elapsedSeconds <= manifest.warmupSeconds + 3
    && measuredWorkload.at(-1)?.elapsedSeconds >= manifest.warmupSeconds + manifest.measureSeconds - 3
    && measuredWorkload.every((record, i) => workloadReady(record)
      && (i === 0 || (record.elapsedSeconds > measuredWorkload[i - 1].elapsedSeconds
        && record.elapsedSeconds - measuredWorkload[i - 1].elapsedSeconds <= 3))),
    'Loaded assets or physical windows changed or were not observed during measurement');
  require(processSamples.length >= 2, 'Process sampling is missing');
  require(processSamples.every(sample => Array.isArray(sample.competingCompilers)),
    'Compiler contention sampling is incomplete');
  const contention = compilerContention(processSamples);
  require(!contention.observed, 'Concurrent compiler/build process observed');
  const measuredProcesses = processSamples.filter(sample => sample.elapsedSeconds >= manifest.warmupSeconds
    && sample.elapsedSeconds <= manifest.warmupSeconds + manifest.measureSeconds);
  const background = backgroundCpu(measuredProcesses, [manifest.pid, manifest.harnessPid]);
  if (Number.isFinite(manifest.maxBackgroundCpuCores)) {
    require(measuredProcesses.every(s => Array.isArray(s.processes)) && background !== null
      && measuredProcesses.at(-1).elapsedSeconds - measuredProcesses[0].elapsedSeconds >= manifest.measureSeconds - 3,
      'Background CPU sampling is incomplete');
    require(background <= manifest.maxBackgroundCpuCores, 'Background CPU exceeded the configured quiet-run allowance');
  }
  require(!/failed to load (?:asset|shader)|path not found|asset.*does not exist|panic(?:ked)? at/i.test(stderr),
    'Content load or runtime error in host log');
  for (const station of manifest.stationIntent || []) {
    const isVisible = d => d.stage === 'console-visible' && d.station === station
      && d.ready === true && d.afk === true && d.rating === 'Backfill'
      && d.lobbyShown === false && d.updateConsoleInstalled === true
      && d.frameWidth > 0 && d.frameHeight > 0;
    const visible = diagnostics.find(record => {
      const d = record.data || record;
      return isVisible(d) && record.elapsedSeconds < manifest.warmupSeconds;
    });
    require(!!visible, 'Active Backfill console was not verified before warm-up: ' + station);
    const last = diagnostics.filter(record => (record.data || record).station === station).at(-1);
    require(last && isVisible(last.data || last)
      && last.elapsedSeconds >= manifest.warmupSeconds + manifest.measureSeconds - 3,
      'Console liveness was not retained through observation: ' + station);
    const heartbeats = diagnostics.filter(record => (record.data || record).station === station
      && record.elapsedSeconds >= manifest.warmupSeconds - 2
      && record.elapsedSeconds <= manifest.warmupSeconds + manifest.measureSeconds);
    require(heartbeats.length >= Math.floor(manifest.measureSeconds / 2)
      && heartbeats.every((record, i) => isVisible(record.data || record)
        && (i === 0 || record.elapsedSeconds - heartbeats[i - 1].elapsedSeconds <= 3)),
      'Console heartbeat gap or non-Backfill state during observation: ' + station);
  }
  return { comparable: reasons.length === 0, reasons, contention, backgroundCpuCores: background };
}

export function readJson(file) {
  return JSON.parse(fs.readFileSync(file, 'utf8').replace(/^\uFEFF/, ''));
}

export function analyzeNativeRun(directory) {
  const manifest = readJson(path.join(directory, 'manifest.json'));
  const readOptional = (file, fallback) => fs.existsSync(path.join(directory, file))
    ? readJson(path.join(directory, file)) : fallback;
  const processSamples = readOptional('process-samples.json', []);
  const diagnostics = readOptional('pane-diagnostics.json', []);
  const frameCapture = readOptional('frames.json', { complete: false });
  const records = nativeRecords(frameCapture);
  const frames = framesInWindow(records, manifest.warmupSeconds, manifest.measureSeconds);
  // Pane/process samples use the harness launch clock; frame samples use the
  // observer clock after App construction. Translate before readiness gating.
  const offset = (frameCapture.startedUnixMs - manifest.startedUnixMs) / 1000;
  if (Number.isFinite(offset)) {
    for (const diagnostic of diagnostics) diagnostic.elapsedSeconds -= offset;
    for (const sample of processSamples) sample.elapsedSeconds -= offset;
  }
  manifest.frameCaptureComplete = frameCapture.complete === true;
  manifest.completedObservation = frameCapture.complete === true;
  const stderr = fs.readFileSync(path.join(directory, 'stderr.log'), 'utf8');
  const workload = frameCapture.workload || [];
  const validation = validateRun({ manifest, processSamples, diagnostics, frames, workload, stderr });
  return {
    manifest, validation, frame: summarizeSamples(frames),
    fixedTicksPerFrame: summarizeSamples(records.filter(r => r.elapsedSeconds - r.frameMs / 1000 >= manifest.warmupSeconds
      && r.elapsedSeconds <= manifest.warmupSeconds + manifest.measureSeconds).map(r => r.fixedTicks)),
    caveat: 'App frame cadence, not GPU time. Pane iteration and main-frame work have different denominators.',
    processSamples, diagnostics, workload,
  };
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (process.argv.length !== 3) throw new Error('Usage: node scripts/profile-analysis.mjs <run-directory>');
  const directory = path.resolve(process.argv[2]);
  const result = analyzeNativeRun(directory);
  fs.writeFileSync(path.join(directory, 'summary.json'), JSON.stringify(result, null, 2) + '\n');
  console.log(JSON.stringify({ frame: result.frame, validation: result.validation }, null, 2));
  if (!result.validation.comparable) process.exitCode = 2;
}
