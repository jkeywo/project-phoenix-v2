#!/usr/bin/env node
// #1543 measurement reducer. Producers supply monotonic timestamps from their
// own clocks. A correlation id never makes two device clocks interchangeable.
import { readFile, writeFile } from 'node:fs/promises';
import { pathToFileURL } from 'node:url';
import path from 'node:path';

export const FORMAT = 'phoenix-t5-performance-events-v1';
const KINDS = new Set(['input', 'authoritative_applied', 'applied_receipt', 'tick',
  'pause_start', 'pause_end', 'fault', 'loss_detected', 'restore_commit',
  'progress_resumed', 'digest_verified']);
const finite = value => typeof value === 'number' && Number.isFinite(value) && value >= 0;
const key = event => `${event.clock}\0${event.peer}\0${event.correlation}`;
const quantile = (sorted, fraction) => sorted[Math.ceil(fraction * sorted.length) - 1];
export function distribution(values) {
  if (!values.length) return null;
  const sorted = [...values].sort((a, b) => a - b);
  return { count: sorted.length, p50_ms: quantile(sorted, .5),
    p95_ms: quantile(sorted, .95), p99_ms: quantile(sorted, .99), max_ms: sorted.at(-1) };
}
export function validateTrace(trace) {
  if (trace?.format !== FORMAT || !Array.isArray(trace.events)) throw new Error('Invalid performance event format');
  if (!trace.provenance || !/^[0-9a-f]{40}$/.test(trace.provenance.revision)
      || typeof trace.provenance.content !== 'string'
      || !trace.provenance.content || !trace.provenance.runtime
      || !trace.provenance.artifactHashes || !trace.provenance.profile) {
    throw new Error('Revision, content, runtime, artifact hashes and profile are required');
  }
  const hashes = Object.values(trace.provenance.artifactHashes);
  if (!hashes.length || hashes.some(hash => !/^[0-9a-f]{64}$/.test(hash))) {
    throw new Error('Artifact hashes must be SHA-256 digests');
  }
  if (!finite(trace.expectedTickMs) || trace.expectedTickMs <= 0) throw new Error('Expected tick interval is required');
  for (const event of trace.events) {
    if (!event || !KINDS.has(event.kind) || typeof event.clock !== 'string'
        || !event.clock || typeof event.peer !== 'string' || !event.peer
        || !finite(event.ms)) throw new Error('Malformed monotonic event');
    if (['input', 'authoritative_applied', 'applied_receipt', 'fault',
      'loss_detected', 'restore_commit', 'progress_resumed', 'digest_verified'].includes(event.kind)
        && (typeof event.correlation !== 'string' || !event.correlation)) {
      throw new Error(`${event.kind} requires a correlation`);
    }
    if (['authoritative_applied', 'tick', 'progress_resumed', 'digest_verified'].includes(event.kind)
        && !Number.isSafeInteger(event.tick)) throw new Error(`${event.kind} requires an observed tick`);
    if (event.kind === 'digest_verified' && event.agreed !== true) throw new Error('Digest verification requires agreement');
  }
  return trace;
}
const elapsed = (start, end) => end.ms >= start.ms ? end.ms - start.ms : null;
function pairs(events, first, last) {
  const starts = new Map(), durations = [], incomplete = [];
  for (const event of events) {
    const id = key(event);
    if (event.kind === first) {
      if (starts.has(id)) throw new Error(`Duplicate ${first}: ${event.correlation}`);
      starts.set(id, event);
    } else if (event.kind === last) {
      const start = starts.get(id);
      if (!start) { incomplete.push({ correlation: event.correlation, reason: `missing ${first}` }); continue; }
      const ms = elapsed(start, event);
      if (ms === null) throw new Error(`Reversed ${first}/${last} clock order`);
      durations.push({ correlation: event.correlation, clock: event.clock,
        peer: start.peer, tick: event.tick, ms });
      starts.delete(id);
    }
  }
  for (const event of starts.values()) incomplete.push({ correlation: event.correlation, reason: `missing ${last}` });
  return { samples: durations, incomplete };
}
function stalls(events, expectedTickMs) {
  const byClock = new Map(), samples = [], observers = [];
  for (const event of events) {
    const observer = `${event.clock}\0${event.peer}`;
    if (!byClock.has(observer)) byClock.set(observer, []);
    byClock.get(observer).push(event);
  }
  for (const [observer, rows] of byClock) {
    let pauseStart = null, previous = null, firstTick = null, lastTick = null;
    const planned = [];
    for (const event of rows) {
      if (event.kind === 'pause_start') {
        if (pauseStart !== null) throw new Error('Overlapping pause events');
        pauseStart = event.ms;
      }
      if (event.kind === 'pause_end') {
        if (pauseStart === null) throw new Error('Pause end without start');
        planned.push([pauseStart, event.ms]); pauseStart = null;
      }
    }
    if (pauseStart !== null) throw new Error('Unclosed pause cannot establish unplanned stalls');
    for (const event of rows) {
      if (event.kind !== 'tick') continue;
      firstTick ??= event.ms;
      lastTick = event.ms;
      if (previous) {
        if (event.tick !== previous.tick + 1) throw new Error('Noncontiguous tick observations cannot establish stalls');
        const gap = elapsed(previous, event);
        if (gap === null) throw new Error('Reversed tick clock order');
        const plannedMs = planned.reduce((sum, [begin, end]) =>
          sum + Math.max(0, Math.min(end, event.ms) - Math.max(begin, previous.ms)), 0);
        const unplannedExcessMs = Math.max(0, gap - plannedMs - expectedTickMs);
        if (gap > expectedTickMs || plannedMs > 0) samples.push({ clock: event.clock, peer: event.peer,
          from_tick: previous.tick, to_tick: event.tick, gap_ms: gap,
          planned_ms: plannedMs, unplanned_excess_ms: unplannedExcessMs });
      }
      previous = event;
    }
    const unplanned = samples.filter(row => `${row.clock}\0${row.peer}` === observer)
      .map(row => row.unplanned_excess_ms);
    const excess = unplanned.reduce((sum, value) => sum + value, 0);
    const duration = firstTick === null ? 0 : lastTick - firstTick;
    observers.push({ clock: rows[0].clock, peer: rows[0].peer,
      measured_clock_ms: duration, unplanned_excess_ms: excess,
      unplanned_fraction: duration > 0 ? excess / duration : null,
      longest_unplanned_ms: unplanned.length ? Math.max(...unplanned) : null });
  }
  return { samples, per_observer: observers,
    worst_observer_fraction: observers.reduce((worst, row) =>
      row.unplanned_fraction === null ? worst : Math.max(worst ?? 0, row.unplanned_fraction), null),
    longest_unplanned_ms: observers.reduce((worst, row) =>
      Math.max(worst, row.longest_unplanned_ms ?? 0), 0) };
}
export function summarize(trace) {
  validateTrace(trace);
  const events = [...trace.events].sort((a, b) => a.clock.localeCompare(b.clock) || a.ms - b.ms);
  const application = pairs(events, 'input', 'authoritative_applied');
  const receipt = pairs(events, 'input', 'applied_receipt');
  // Recovery is measured at one observer. These phases must share its clock;
  // a host's restore timestamp cannot be subtracted from a client's fault time.
  const detection = pairs(events, 'fault', 'loss_detected');
  const progress = pairs(events, 'fault', 'progress_resumed');
  const verified = pairs(events, 'fault', 'digest_verified');
  const restores = events.filter(event => event.kind === 'restore_commit');
  return { format: 'phoenix-t5-performance-summary-v1', provenance: trace.provenance,
    clock_rule: 'Durations use only one producer monotonic clock; receipts include return delivery.',
    input_to_authoritative_application: { ...application,
      distribution: distribution(application.samples.map(row => row.ms)) },
    input_to_applied_receipt: { ...receipt,
      distribution: distribution(receipt.samples.map(row => row.ms)) },
    stalls: stalls(events, trace.expectedTickMs),
    recovery: { fault_to_loss_detection: { ...detection,
      distribution: distribution(detection.samples.map(row => row.ms)) },
      fault_to_progress: { ...progress,
        distribution: distribution(progress.samples.map(row => row.ms)) },
      fault_to_verified_digest: { ...verified,
        distribution: distribution(verified.samples.map(row => row.ms)) },
      restore_commits: restores.map(({ peer, clock, correlation, ms, tick }) =>
        ({ peer, clock, correlation, ms, tick })) } };
}
if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  const [input, output] = process.argv.slice(2);
  if (!input || !output) throw new Error('Usage: node scripts/fleet-performance-evidence.mjs trace.json summary.json');
  const summary = summarize(JSON.parse(await readFile(input, 'utf8')));
  await writeFile(output, JSON.stringify(summary, null, 2) + '\n');
}
