import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { summarizeSamples, compilerContention, backgroundCpu, readJson, analyzeNativeRun } from './profile-analysis.mjs';

export function summarizeSystems(artifact) {
  const withinWindow = span => !artifact.windowSeconds || (span.start_ns >= artifact.windowSeconds[0] * 1e9
    && span.end_ns <= artifact.windowSeconds[1] * 1e9);
  const systems = artifact.systems.map(row => {
    const spans = row.counts.spans.filter(withinWindow);
    const run = spans.filter(span => !span.deferred);
    const deferred = spans.filter(span => span.deferred);
    return {
    name: row.name, schedule: row.schedule, category: row.category,
    calls: run.length, accumulatedMs: run.reduce((sum, span) => sum + span.end_ns - span.start_ns, 0) / 1e6,
    maxMs: run.reduce((max, span) => Math.max(max, span.end_ns - span.start_ns), 0) / 1e6,
    deferredCalls: deferred.length, deferredMs: deferred.reduce((sum, span) => sum + span.end_ns - span.start_ns, 0) / 1e6,
    unfilteredTotals: { calls: row.counts.calls, accumulatedMs: row.counts.nanos / 1e6,
      deferredCalls: row.counts.deferred_calls, deferredMs: row.counts.deferred_ns / 1e6 },
  }; }).sort((a, b) => b.accumulatedMs - a.accumulatedMs);
  const slowUpdates = [...artifact.updates].sort((a, b) => b.durationMs - a.durationMs).slice(0, 10)
    .map(update => ({ ...update, spans: artifact.systems.flatMap(row => row.counts.spans
      .filter(span => span.update === update.update).map(span => ({
        name: row.name, schedule: row.schedule, category: row.category, deferred: span.deferred,
        durationMs: (span.end_ns - span.start_ns) / 1e6,
      }))).sort((a, b) => b.durationMs - a.durationMs).slice(0, 15) }));
  const paths = new Map();
  for (const sample of artifact.renderDiagnostics?.samples || []) {
    if (!paths.has(sample.path)) paths.set(sample.path, []);
    paths.get(sample.path).push(sample.value);
  }
  const renderDiagnostics = Object.fromEntries([...paths].sort(([a], [b]) => a.localeCompare(b))
    .map(([name, values]) => [name, { unit: /elapsed_(gpu|cpu)$/.test(name) ? 'ms' : 'count', ...summarizeSamples(values) }]));
  return { systems, slowUpdates, update: summarizeSamples(artifact.updates.map(update => update.durationMs)),
    renderDiagnostics, gpuObserved: [...paths.keys()].some(name => name.endsWith('/elapsed_gpu')),
    continuation: artifact.continuation,
    caveat: 'System wall spans overlap and include waits. Deferred ExtractSchedule spans are nested in ExtractCommands; never add both as exclusive cost. GPU paths are asynchronous delivered samples and nested paths overlap.' };
}

export function analyzeSystemsRun(directory) {
  const manifest = readJson(path.join(directory, 'manifest.json'));
  const artifact = readJson(path.join(directory, 'artifact', 'systems.json'));
  const processes = readJson(path.join(directory, 'process-samples.json'));
  const reasons = [];
  if (manifest.exitCode !== 0 || manifest.timedOut) reasons.push('Capture did not exit successfully');
  if (!manifest.buildReceiptVerified || !manifest.provenanceUnchanged || !manifest.isolatedState
    || !/^[0-9a-f]{40}$/i.test(manifest.sourceRevision || '') || !/^[0-9a-f]{64}$/i.test(manifest.binarySha256 || '')) reasons.push('Provenance or isolation is incomplete');
  if (artifact.traceTruncated || artifact.renderDiagnostics?.truncated) reasons.push('Raw trace was truncated');
  if (artifact.renderDiagnostics?.history_may_be_truncated) reasons.push('A delivered render diagnostic path filled its entire retained history');
  if (compilerContention(processes).observed) reasons.push('Concurrent compilation observed');
  const background = backgroundCpu(processes, [manifest.pid, manifest.harnessPid]);
  // Native validation below uses only observation-time CPU, translated to the
  // frame observer's clock. Headless has an update-index warm-up, so retain the
  // conservative whole-process sampled load as a separate limit.
  if (manifest.runtime !== 'native' && (background === null || background > manifest.maxBackgroundCpuCores)) reasons.push('Background CPU exceeds quiet-run allowance or was not sampled');
  if (manifest.mode !== artifact.mode || manifest.runtime !== artifact.runtime || artifact.world !== `assets/worlds/${manifest.world}.toml`) reasons.push('Requested workload does not match capture');
  const stderr = fs.readFileSync(path.join(directory, 'stderr.log'), 'utf8');
  if (/failed to load (?:asset|shader)|path not found|asset.*does not exist|panic(?:ked)? at/i.test(stderr)) reasons.push('Content or runtime error');
  let native;
  if (manifest.runtime === 'native') {
    // The separate renderer attribution example carries no Ultralight surfaces
    // and installs only the frame collector, identically in both control modes.
    native = analyzeNativeRun(directory, { surfaceAttribution: false });
    reasons.push(...native.validation.reasons);
  } else if (!artifact.updates.length || !/^[0-9a-f]{16}$/i.test(artifact.continuation?.digest || '')) reasons.push('Headless continuation or raw updates missing');
  return { manifest, validation: { comparable: reasons.length === 0, reasons: [...new Set(reasons)], backgroundCpuCores: background },
    ...summarizeSystems(artifact), native };
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const directory = path.resolve(process.argv[2]);
  const result = analyzeSystemsRun(directory);
  fs.writeFileSync(path.join(directory, 'summary.json'), JSON.stringify(result, null, 2) + '\n');
  console.log(JSON.stringify({ validation: result.validation, update: result.update, gpuObserved: result.gpuObserved }));
  if (!result.validation.comparable) process.exitCode = 2;
}
