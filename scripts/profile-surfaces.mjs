// Integer-nanosecond surface events (#1405). No per-view raster cost is inferred.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const eventCounters = {
  main_frame: 'main_frames', main_pass: 'main_passes', iteration: 'worker_iterations',
  lifecycle: 'lifecycle_events', hud_slot: 'hud_slot_revisions', push: 'push_batches',
  copy: 'copy_decisions', produced: 'produced', drained: 'drained', extracted: 'extracted',
  deferred: 'deferred_attempts', promoted_full: 'promoted_full', uploaded: 'uploaded', discarded: 'discarded',
};
const causes = ['initial', 'resize', 'reveal', 'bridge_push', 'hud_push', 'copy_retry', 'buffer_retry'];
const losses = ['closed', 'stale_epoch', 'superseded_deferral', 'deferral_exhausted', 'refused_layout', 'no_renderer', 'buffer_dropped'];
const frameEvents = new Set(['produced', 'drained', 'extracted', 'deferred', 'promoted_full', 'uploaded', 'discarded']);
const workerPhases = ['update_ns', 'pump_ns', 'render_ns', 'copy_ns', 'publish_ns', 'total_ns'];
const numericFields = {
  main_frame: [], main_pass: ['drain_ns', 'queue_ns', 'frames'], iteration: workerPhases,
  lifecycle: [], hud_slot: ['revision'], push: ['applied', 'failed', 'deferred_messages', 'duration_ns'],
  copy: ['copied_pixels', 'duration_ns'], produced: ['pixels'], drained: ['age_ns'], extracted: ['age_ns'],
  deferred: ['age_ns', 'attempts'], promoted_full: ['pixels'], uploaded: ['age_ns', 'pixels', 'write_texture_ns'],
  discarded: ['age_ns'],
};

function integer(value, name) {
  if (!Number.isSafeInteger(value) || value < 0) throw new Error(`Invalid exact integer: ${name}`);
  return value;
}
function add(row, key, amount = 1) {
  row[key] = integer((row[key] || 0) + integer(amount, key), key);
}
function surfaceKey(surface) {
  if (!surface || !['console', 'lobby', 'hud'].includes(surface.kind)
      || typeof surface.visible !== 'boolean' || !(surface.device_scale > 0)
      || !Number.isFinite(surface.device_scale)) throw new Error('Invalid surface identity');
  for (const field of ['id', 'epoch', 'width', 'height']) integer(surface[field], field);
  return JSON.stringify([surface.id, surface.epoch, surface.kind, surface.width,
    surface.height, surface.device_scale, surface.visible]);
}
// Schema 1 captures made before the rectangle increment have only pixel
// counts. Do not infer a shape for them or relabel copied pixels as dirtiness.
function validateCopyRectangles(event) {
  const fields = ['dirty_rect', 'copied_rect'];
  if (fields.every(key => !Object.hasOwn(event, key))) return;
  if (fields.some(key => !Object.hasOwn(event, key))) throw new Error('Incomplete copy rectangles');
  for (const key of fields) {
    const rect = event[key];
    if (rect === null) continue;
    if (!rect || typeof rect !== 'object') throw new Error(`Invalid ${key}`);
    for (const edge of ['left', 'top', 'right', 'bottom']) integer(rect[edge], `${key}.${edge}`);
    if (rect.right < rect.left || rect.bottom < rect.top
        || rect.right > event.surface.width || rect.bottom > event.surface.height)
      throw new Error(`Out-of-bounds ${key}`);
    const pixels = integer((rect.right - rect.left) * (rect.bottom - rect.top), `${key} area`);
    if (pixels !== event[key === 'dirty_rect' ? 'dirty_pixels' : 'copied_pixels'])
      throw new Error(`Pixel count disagrees with ${key}`);
  }
  if ((event.dirty_rect === null) !== (event.dirty_pixels === null)
      || (event.copied_rect === null && event.copied_pixels !== 0))
    throw new Error('Copy rectangle knowledge disagrees with pixel count');
  if (event.outcome === 'copied' ? event.copied_rect === null || event.copied_pixels === 0 : event.copied_rect !== null)
    throw new Error('Copy rectangle disagrees with outcome');
  if ((event.forced || ['failed', 'buffer_starved'].includes(event.outcome)) && event.dirty_rect !== null)
    throw new Error('Unobserved dirty rectangle must remain unknown');
}
function timing(values) {
  if (!values.length) return null;
  const sorted = [...values].sort((a, b) => a - b);
  const sum = sorted.reduce((total, value) => integer(total + value, 'timing sum'), 0);
  return { count: sorted.length, sumNs: sum, meanNs: sum / sorted.length,
    p50Ns: sorted[Math.ceil(sorted.length * 0.5) - 1],
    p95Ns: sorted[Math.ceil(sorted.length * 0.95) - 1],
    p99Ns: sorted[Math.ceil(sorted.length * 0.99) - 1], maxNs: sorted.at(-1) };
}
function per(totals, count) {
  return Object.fromEntries(Object.entries(totals).map(([key, value]) => [key, count ? value / count : null]));
}
function group(surface) {
  return { surface, eventCounts: {}, push: { batches: 0, applied: 0, failed: 0, deferredMessages: 0,
    durationNs: 0, repeatedHudApplications: 0, hudRevisions: new Set(), channels: {} },
  copy: { outcomes: {}, copiedPixels: 0, knownDirtyPixels: 0, unknownDirtyDecisions: 0, durationNs: 0 },
  produced: { count: 0, pixels: 0, forced: 0, fullCopyReasons: {} },
  upload: { count: 0, pixels: 0, bytes: 0, full: 0, writeTextureNs: 0 },
  discard: { count: 0, reasons: {} }, deferredAttempts: 0, promotedFullPixels: 0,
  producedInWindow: { produced: 0, uploadedWithinWindow: 0, discardedWithinWindow: 0,
    uploadedAfterWindow: 0, discardedAfterWindow: 0, inFlightAtWindowEnd: 0, inFlightAtCaptureClose: 0 },
  earlierBoundary: { uploaded: 0, uploadedPixels: 0, discarded: 0 },
  ages: { drained: [], extracted: [], deferred: [], uploaded: [], discarded: [] } };
}

/** Window offsets use the capture's monotonic origin; integrate with a shared
 * installer origin, never pass another collector's offsets without conversion.
 * Point events use [start, end). Timed main passes/worker iterations crossing
 * the start boundary are excluded from phase averages and counted separately. */
export function analyzeSurfaceCapture(artifact, { warmupSeconds, measureSeconds }) {
  if (artifact?.schema !== 1 || !Array.isArray(artifact.events) || !artifact.totals)
    throw new Error('Expected surface capture schema 1');
  if (!Number.isFinite(warmupSeconds) || warmupSeconds < 0
      || !Number.isFinite(measureSeconds) || measureSeconds <= 0) throw new Error('Invalid surface observation window');
  const startNs = integer(Math.round(warmupSeconds * 1e9), 'window start');
  const endNs = integer(Math.round((warmupSeconds + measureSeconds) * 1e9), 'window end');
  if (endNs <= startNs) throw new Error('Empty surface observation window');
  const elapsedNs = integer(artifact.elapsed_ns, 'elapsed_ns');
  const omitted = integer(artifact.omitted_events, 'omitted_events');
  const inFlight = integer(artifact.in_flight_at_close, 'in_flight_at_close');
  const problems = [];
  if (artifact.successful_exit !== true) problems.push('capture did not finish with a successful exit');
  if (omitted) problems.push('raw events were truncated');
  if (elapsedNs < endNs) problems.push('capture ended before the observation window');
  if (artifact.events.some(e => e.event === 'lifecycle' && e.action === 'worker_failed'))
    problems.push('pane worker failed');
  const report = { comparable: problems.length === 0, problems,
    window: { startNs, endNs, convention: '[start, end)' },
    capture: { elapsedNs, omittedEvents: omitted, inFlightAtClose: inFlight, totals: artifact.totals },
    main: { frames: 0, passes: 0, boundaryPassesExcluded: 0, totalsNs: { drain_ns: 0, queue_ns: 0 } },
    worker: { iterations: 0, boundaryIterationsExcluded: 0,
      totalsNs: Object.fromEntries([...workerPhases, 'unattributed_ns'].map(key => [key, 0])) },
    hudSlotChanges: 0, surfaces: [] };
  // Truncated detail cannot establish ownership/cohort outcomes. Preserve the
  // header's exact global counts, but do not manufacture partial comparisons.
  if (omitted) return report;
  const events = [...artifact.events].sort((a, b) => a.at_ns - b.at_ns);
  const frames = new Map(), groups = new Map(), revisions = new Map(), observed = {};
  const inside = e => e.at_ns >= startNs && e.at_ns < endNs;
  function rowFor(surface) {
    const key = surfaceKey(surface);
    if (!groups.has(key)) groups.set(key, group(surface));
    return groups.get(key);
  }
  for (const event of events) {
    integer(event.at_ns, 'event timestamp');
    if (event.at_ns > elapsedNs || !Object.hasOwn(eventCounters, event.event)) throw new Error('Invalid surface event');
    numericFields[event.event].forEach(key => integer(event[key], key));
    if (event.surface) surfaceKey(event.surface);
    if (['push', 'copy'].includes(event.event) && !event.surface) throw new Error('Missing surface identity');
    if (event.event === 'push' && !['gamepad', 'bridge_pump'].includes(event.channel)) throw new Error('Invalid push channel');
    if (event.event === 'copy') {
      if (!['copied', 'clean', 'failed', 'buffer_starved'].includes(event.outcome)) throw new Error('Invalid copy outcome');
      if (event.dirty_pixels !== null) integer(event.dirty_pixels, 'dirty_pixels');
      validateCopyRectangles(event);
    }
    if (event.event === 'discarded' && !losses.includes(event.reason)) throw new Error('Invalid discard reason');
    if (['copy', 'produced'].includes(event.event) && typeof event.forced !== 'boolean') throw new Error('Invalid forced flag');
    if (event.event === 'uploaded' && typeof event.full !== 'boolean') throw new Error('Invalid full flag');
    add(observed, eventCounters[event.event]);
    if (frameEvents.has(event.event)) {
      integer(event.frame, 'frame sequence');
      const key = surfaceKey(event.surface);
      if (event.event === 'produced') {
        if (frames.has(event.frame)) throw new Error('Duplicate frame production');
        frames.set(event.frame, { produced: event, key, terminal: null });
      } else {
        const frame = frames.get(event.frame);
        if (!frame || frame.key !== key || frame.terminal) throw new Error('Broken frame ownership/lifetime');
        if (event.event !== 'promoted_full') {
          integer(event.age_ns, 'frame age');
          if (event.age_ns !== event.at_ns - frame.produced.at_ns) throw new Error('Frame age disagrees with its origin');
        }
        if (['uploaded', 'discarded'].includes(event.event)) frame.terminal = event;
      }
    }
    let repeated = 0;
    if (event.event === 'push' && event.surface?.kind === 'hud' && event.applied > 0) {
      // Visibility changes split reporting rows, but do not create a new view.
      const key = JSON.stringify([event.surface.id, event.surface.epoch]);
      if (!revisions.has(key)) revisions.set(key, new Set());
      const seen = revisions.get(key);
      integer(event.revision, 'applied HUD revision');
      repeated = event.applied - (seen.has(event.revision) ? 0 : 1);
      seen.add(event.revision);
    }
    if (!inside(event)) continue;
    if (event.event === 'main_frame') { report.main.frames++; continue; }
    if (event.event === 'hud_slot') { report.hudSlotChanges++; continue; }
    if (event.event === 'main_pass' || event.event === 'iteration') {
      const main = event.event === 'main_pass';
      const fields = main ? ['drain_ns', 'queue_ns'] : workerPhases;
      fields.forEach(key => integer(event[key], key));
      const duration = main ? event.drain_ns + event.queue_ns : event.total_ns;
      const target = main ? report.main : report.worker;
      if (event.at_ns - duration < startNs) {
        target[main ? 'boundaryPassesExcluded' : 'boundaryIterationsExcluded']++;
        continue;
      }
      target[main ? 'passes' : 'iterations']++;
      fields.forEach(key => add(target.totalsNs, key, event[key]));
      if (!main) add(target.totalsNs, 'unattributed_ns', event.total_ns
        - workerPhases.filter(key => key !== 'total_ns').reduce((sum, key) => sum + event[key], 0));
      continue;
    }
    if (!event.surface) continue;
    const row = rowFor(event.surface);
    add(row.eventCounts, event.event);
    if (Object.hasOwn(row.ages, event.event)) row.ages[event.event].push(event.age_ns);
    switch (event.event) {
      case 'push': {
        add(row.push, 'batches');
        for (const key of ['applied', 'failed']) add(row.push, key, event[key]);
        add(row.push, 'deferredMessages', event.deferred_messages);
        add(row.push, 'durationNs', event.duration_ns);
        add(row.push, 'repeatedHudApplications', repeated);
        const channel = row.push.channels[event.channel] ||= { batches: 0, applied: 0, failed: 0 };
        add(channel, 'batches');
        for (const key of ['applied', 'failed']) add(channel, key, event[key]);
        if (event.surface.kind === 'hud' && event.applied > 0) row.push.hudRevisions.add(event.revision);
        break;
      }
      case 'copy':
        add(row.copy.outcomes, event.outcome);
        add(row.copy, 'copiedPixels', event.copied_pixels);
        add(row.copy, 'durationNs', event.duration_ns);
        if (event.dirty_pixels === null) add(row.copy, 'unknownDirtyDecisions');
        else add(row.copy, 'knownDirtyPixels', event.dirty_pixels);
        break;
      case 'produced':
        add(row.produced, 'count'); add(row.produced, 'pixels', event.pixels);
        if (event.forced) add(row.produced, 'forced');
        for (const cause of causes) if (event.reasons?.[cause]) add(row.produced.fullCopyReasons, cause);
        break;
      case 'uploaded':
        add(row.upload, 'count'); add(row.upload, 'pixels', event.pixels);
        add(row.upload, 'bytes', event.pixels * 4); add(row.upload, 'writeTextureNs', event.write_texture_ns);
        if (event.full) add(row.upload, 'full');
        if (frames.get(event.frame).produced.at_ns < startNs) {
          add(row.earlierBoundary, 'uploaded'); add(row.earlierBoundary, 'uploadedPixels', event.pixels);
        }
        break;
      case 'discarded':
        add(row.discard, 'count'); add(row.discard.reasons, event.reason);
        if (frames.get(event.frame).produced.at_ns < startNs) add(row.earlierBoundary, 'discarded');
        break;
      case 'deferred': add(row, 'deferredAttempts'); break;
      case 'promoted_full': add(row, 'promotedFullPixels', event.pixels); break;
    }
  }
  for (const key of Object.values(eventCounters)) {
    if (integer(artifact.totals[key] ?? 0, key) !== (observed[key] || 0)) throw new Error(`Raw/count mismatch: ${key}`);
  }
  const unfinished = [...frames.values()].filter(frame => !frame.terminal).length;
  if (unfinished !== inFlight) throw new Error('In-flight header disagrees with frame lifetimes');
  for (const frame of frames.values()) {
    if (!inside(frame.produced)) continue;
    const cohort = rowFor(frame.produced.surface).producedInWindow;
    add(cohort, 'produced');
    if (!frame.terminal || frame.terminal.at_ns >= endNs) add(cohort, 'inFlightAtWindowEnd');
    if (!frame.terminal) add(cohort, 'inFlightAtCaptureClose');
    else add(cohort, `${frame.terminal.event}${inside(frame.terminal) ? 'WithinWindow' : 'AfterWindow'}`);
  }
  for (const row of groups.values()) {
    row.ages = Object.fromEntries(Object.entries(row.ages).map(([key, values]) => [key, timing(values)]));
    row.push.hudRevisions = [...row.push.hudRevisions].sort((a, b) => a - b);
    report.surfaces.push(row);
  }
  report.surfaces.sort((a, b) => {
    const left = surfaceKey(a.surface), right = surfaceKey(b.surface);
    return left < right ? -1 : left > right ? 1 : 0;
  });
  report.main.perFrameNs = per(report.main.totalsNs, report.main.frames);
  report.worker.perIterationNs = per(report.worker.totalsNs, report.worker.iterations);
  return report;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [input, warmup, duration, output] = process.argv.slice(2);
  if (!input || warmup === undefined || duration === undefined)
    throw new Error('Usage: profile-surfaces.mjs <capture.json> <warmup-seconds> <measure-seconds> [new-output.json]');
  const report = analyzeSurfaceCapture(JSON.parse(fs.readFileSync(input, 'utf8').replace(/^\uFEFF/, '')),
    { warmupSeconds: Number(warmup), measureSeconds: Number(duration) });
  const json = JSON.stringify(report, null, 2) + '\n';
  if (output) fs.writeFileSync(output, json, { flag: 'wx' });
  else process.stdout.write(json);
  if (!report.comparable) process.exitCode = 2;
}
