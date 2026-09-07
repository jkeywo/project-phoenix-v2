import { describe, expect, it } from 'vitest';
import { analyzeSurfaceCapture } from '../../scripts/profile-surfaces.mjs';

const surface = (patch = {}) => ({ id: 7, epoch: 2, kind: 'hud', width: 100, height: 50,
  device_scale: 1.5, visible: true, ...patch });
const event = (kind, seconds, fields = {}) => ({ event: kind, at_ns: seconds * 1e9,
  frame: null, surface: null, ...fields });
const produced = (frame, seconds, identity = surface()) => event('produced', seconds,
  { frame, surface: identity, pixels: 5000, forced: true, reasons: { hud_push: true }, hud_revision: 1 });
const uploaded = (frame, seconds, producedSeconds, identity = surface()) => event('uploaded', seconds,
  { frame, surface: identity, age_ns: (seconds - producedSeconds) * 1e9,
    pixels: 5000, full: true, write_texture_ns: 123 });
const discarded = (frame, seconds, producedSeconds, reason = 'stale_epoch', identity = surface()) =>
  event('discarded', seconds, { frame, surface: identity, age_ns: (seconds - producedSeconds) * 1e9, reason });
const push = (seconds, revision, identity = surface(), applied = 1, failed = 0) => event('push', seconds,
  { surface: identity, channel: 'bridge_pump', revision, applied, failed, deferred_messages: 0, duration_ns: 42 });
const names = { produced: 'produced', uploaded: 'uploaded', discarded: 'discarded',
  push: 'push_batches', main_frame: 'main_frames', main_pass: 'main_passes', iteration: 'worker_iterations',
  hud_slot: 'hud_slot_revisions', lifecycle: 'lifecycle_events', copy: 'copy_decisions',
  drained: 'drained', extracted: 'extracted', deferred: 'deferred_attempts', promoted_full: 'promoted_full' };
function capture(events, patch = {}) {
  const totals = {};
  for (const e of events) totals[names[e.event]] = (totals[names[e.event]] || 0) + 1;
  return { schema: 1, successful_exit: true, elapsed_ns: 25e9, omitted_events: 0,
    in_flight_at_close: (totals.produced || 0) - (totals.uploaded || 0) - (totals.discarded || 0),
    totals, events, ...patch };
}
const reduce = data => analyzeSurfaceCapture(data, { warmupSeconds: 10, measureSeconds: 10 });

describe('surface capture reduction', () => {
  it('sorts raw events and separates boundary uploads, within-window outcomes and unfinished lifetimes', () => {
    const input = [produced(0, 9), uploaded(0, 11, 9), produced(1, 12), uploaded(1, 13, 12),
      produced(2, 14), discarded(2, 16, 14), produced(3, 17), uploaded(3, 21, 17), produced(4, 18)];
    const report = reduce(capture([...input].reverse()));
    expect(report.comparable).toBe(true);
    const row = report.surfaces[0];
    expect(row.upload).toEqual({ count: 2, pixels: 10000, bytes: 40000, full: 2, writeTextureNs: 246 });
    expect(row.earlierBoundary).toEqual({ uploaded: 1, uploadedPixels: 5000, discarded: 0 });
    expect(row.producedInWindow).toEqual({ produced: 4, uploadedWithinWindow: 1, discardedWithinWindow: 1,
      uploadedAfterWindow: 1, discardedAfterWindow: 0, inFlightAtWindowEnd: 2, inFlightAtCaptureClose: 1 });
    expect(row.discard.reasons).toEqual({ stale_epoch: 1 });
    expect(row.ages.uploaded).toMatchObject({ count: 2, sumNs: 3e9, meanNs: 1.5e9, maxNs: 2e9 });
    expect(row.produced.fullCopyReasons).toEqual({ hud_push: 4 });
    expect(report.capture.inFlightAtClose).toBe(1);
    expect(input[0].frame).toBe(0);
  });

  it('counts repeated HUD applications from warmup and across visibility but keeps epoch and geometry identities separate', () => {
    const hidden = surface({ visible: false });
    const resized = surface({ epoch: 3, width: 200 });
    const report = reduce(capture([push(8, 1), push(11, 1), push(12, 1, hidden),
      push(13, 2, hidden, 0, 1), push(14, 2, hidden), push(15, 2, resized), push(16, 2, resized),
      event('hud_slot', 13, { revision: 2, has_script: true })]));
    const visible = report.surfaces.find(row => row.surface.epoch === 2 && row.surface.visible);
    const concealed = report.surfaces.find(row => !row.surface.visible);
    const next = report.surfaces.find(row => row.surface.epoch === 3);
    expect(visible.push).toMatchObject({ applied: 1, repeatedHudApplications: 1, hudRevisions: [1] });
    expect(concealed.push).toMatchObject({ applied: 2, failed: 1, repeatedHudApplications: 1, hudRevisions: [1, 2] });
    expect(next.push).toMatchObject({ applied: 2, repeatedHudApplications: 1, hudRevisions: [2] });
    expect(next.surface.width).toBe(200);
    expect(report.hudSlotChanges).toBe(1);
  });

  it('uses main frames and worker iterations as distinct denominators and excludes timed intervals crossing warmup', () => {
    const iteration = (at, total) => event('iteration', at, { update_ns: 1, pump_ns: 2,
      render_ns: 3, copy_ns: 4, publish_ns: 5, total_ns: total });
    const report = reduce(capture([event('main_frame', 10), event('main_frame', 15), event('main_frame', 20),
      event('main_pass', 11, { drain_ns: 20, queue_ns: 10, frames: 1 }),
      iteration(10, 30), iteration(12, 30), iteration(13, 30), iteration(14, 30)]));
    expect(report.main).toMatchObject({ frames: 2, passes: 1, perFrameNs: { drain_ns: 10, queue_ns: 5 } });
    expect(report.worker).toMatchObject({ iterations: 3, boundaryIterationsExcluded: 1,
      totalsNs: { render_ns: 9, total_ns: 90, unattributed_ns: 45 },
      perIterationNs: { render_ns: 3, total_ns: 30, unattributed_ns: 15 } });
    expect(report.surfaces).toEqual([]);
  });

  it('keeps staging starvation separate from produced-frame loss and dirty unknowns separate from zero dirty pixels', () => {
    const copy = (seconds, fields) => event('copy', seconds, { surface: surface(), outcome: 'copied',
      copied_pixels: 5000, dirty_pixels: null, forced: true, reasons: { hud_push: true }, duration_ns: 20, ...fields });
    const report = reduce(capture([copy(11, { outcome: 'buffer_starved', copied_pixels: 0, duration_ns: 0 }),
      copy(12, {}), produced(0, 12), event('deferred', 13, { frame: 0, surface: surface(), age_ns: 1e9, attempts: 1 }),
      copy(14, { outcome: 'clean', copied_pixels: 0, dirty_pixels: 0, forced: false }),
      discarded(0, 15, 12, 'deferral_exhausted')]));
    expect(report.surfaces[0].copy).toEqual({ outcomes: { buffer_starved: 1, copied: 1, clean: 1 },
      copiedPixels: 5000, knownDirtyPixels: 0, unknownDirtyDecisions: 2, durationNs: 40 });
    expect(report.surfaces[0].produced.count).toBe(1);
    expect(report.surfaces[0].discard).toEqual({ count: 1, reasons: { deferral_exhausted: 1 } });
    expect(report.surfaces[0].deferredAttempts).toBe(1);
  });

  it('rejects truncated detail, worker failure and an insufficient observation window', () => {
    const truncated = reduce(capture([produced(0, 11)], { omitted_events: 1 }));
    expect(truncated.comparable).toBe(false);
    expect(truncated.problems).toContain('raw events were truncated');
    expect(truncated.surfaces).toEqual([]);
    const failed = reduce(capture([event('lifecycle', 22, { action: 'worker_failed' })]));
    expect(failed.comparable).toBe(false);
    expect(failed.problems).toContain('pane worker failed');
    expect(reduce(capture([], { elapsed_ns: 19e9 })).comparable).toBe(false);
    expect(reduce(capture([], { successful_exit: false })).comparable).toBe(false);
  });

  it('refuses broken identity, duplicate terminal outcomes, rounded integers and mismatched counters', () => {
    expect(() => reduce(capture([produced(0, 11), uploaded(0, 12, 11, surface({ epoch: 3 }))]))).toThrow('ownership');
    expect(() => reduce(capture([produced(0, 11), uploaded(0, 12, 11), discarded(0, 13, 11)]))).toThrow();
    expect(() => reduce(capture([event('main_frame', 11)], { totals: { main_frames: 2 } }))).toThrow('mismatch');
    expect(() => reduce(capture([], { elapsed_ns: Number.MAX_SAFE_INTEGER + 1 }))).toThrow('exact integer');
    expect(() => reduce(capture([produced(0, 11)], { in_flight_at_close: 0 }))).toThrow('In-flight');
    expect(() => reduce(capture([push(11, 1, null)]))).toThrow('Missing surface');
    expect(() => reduce(capture([push(8, 1, surface(), NaN)]))).toThrow('exact integer');
  });
});
