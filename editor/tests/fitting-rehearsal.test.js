import { expect, it, vi } from 'vitest';
import { createFittingRehearsal, rehearsalChoices, REHEARSAL_WORLDS } from '../fitting-rehearsal.js';
import { defaultFittingPlan, stationFittingPlan } from '../../gui/fitting-plans.js';

const context = () => ({ hull_path: 'assets/entities/alliance_cruiser.toml', content_id: '0000000000000013', slots: [
  { slot: 'fore-beam', system: 'phaser-fore', home_station: 'tactical', default_equipment: 'beam-standard', compatible_equipment: ['beam-standard', 'beam-pulse'] },
  { slot: 'reactor', system: 'power-reactor', home_station: 'engineering', default_equipment: 'reactor-standard', compatible_equipment: ['reactor-standard'] },
] });
function fixture(transform = report => report) {
  const ctx = context(), source = { 'assets/entities/hull.toml': 'exact unsaved bytes' };
  let files, run;
  const starts = [];
  const session = { state: () => ({ run }), poll: vi.fn(), stop: vi.fn(async () => { run = null; }),
    start: vi.fn(async selection => {
      starts.push({ files: structuredClone(files), selection: structuredClone(selection) });
      const report = transform({ revision: 'captured', hull_content_id: ctx.content_id, workload_revision: 'real-workload',
        run_generation: selection.run_generation, selection: { world: selection.world, ship: selection.ship, slot: selection.slot, seed: selection.seed },
        profile: { workload: selection.rehearsal.workload }, choices: rehearsalChoices(selection.rehearsal.plan, ctx),
        fixed_period_secs: 1 / 60, complete: true }, starts.length);
      run = { running: true, revision: report.revision, rehearsal: report };
      source['assets/entities/hull.toml'] = 'later document edit';
    }) };
  const capture = vi.fn(() => source);
  const owner = createFittingRehearsal({ session, capture, useCapture: next => { files = next; } });
  return { owner, session, starts, capture, ctx };
}
it('runs defaults and a Station favourite in independent children from one defensive capture', async () => {
  const f = fixture(), chosen = defaultFittingPlan(f.ctx);
  chosen.sections.tactical.choices['fore-beam'] = 'beam-pulse';
  await f.owner.start(stationFittingPlan(chosen, f.ctx, 'tactical'), f.ctx, 'burst-recovery', 13);
  expect(f.capture).toHaveBeenCalledTimes(1);
  expect(f.starts).toHaveLength(2);
  expect(f.starts[0].files).toEqual(f.starts[1].files);
  expect(f.starts[1].files['assets/entities/hull.toml']).toBe('exact unsaved bytes');
  expect(f.starts[0].selection.world).toBe(REHEARSAL_WORLDS['burst-recovery']);
  expect(f.starts[0].selection.run_generation).not.toBe(f.starts[1].selection.run_generation);
  expect(f.owner.state().results.selected.choices).toEqual({ 'fore-beam': 'beam-pulse', reactor: 'reactor-standard' });
  expect(f.owner.state().results.baseline.choices['fore-beam']).toBe('beam-standard');
  expect(f.session.stop).toHaveBeenCalledTimes(3);
  f.owner.changed(); expect(f.owner.state().results).toBeNull();
});
it.each(['run_generation', 'hull_content_id', 'choices', 'selection', 'revision', 'workload_revision'])('refuses a mismatched %s rather than combining trials', async key => {
  const f = fixture((r, ordinal) => {
    if (ordinal !== 2) return r;
    if (key === 'choices') return { ...r, choices: {} };
    if (key === 'selection') return { ...r, selection: { ...r.selection, seed: 99 } };
    return { ...r, [key]: key === 'run_generation' ? r.run_generation - 1 : 'different' };
  });
  await expect(f.owner.start(defaultFittingPlan(f.ctx), f.ctx, 'sustained-engagement', 7)).rejects.toThrow('stale');
  expect(f.owner.state().results).toBeNull(); expect(f.owner.state().busy).toBe(false);
});
it('retires a cancelled pending child and refuses its late result', async () => {
  const f = fixture();
  let release;
  f.session.start.mockImplementationOnce(() => new Promise(resolve => { release = resolve; }));
  const pending = f.owner.start(defaultFittingPlan(f.ctx), f.ctx, 'sustained-engagement', 7);
  await vi.waitFor(() => expect(release).toBeTypeOf('function'));
  await f.owner.cancel(); release();
  await expect(pending).rejects.toThrow('cancelled');
  expect(f.owner.state().results).toBeNull(); expect(f.owner.state().busy).toBe(false);
});
it('does not stop an ordinary Test when source or plan changes outside a rehearsal', () => {
  const f = fixture(); f.owner.changed(); expect(f.session.stop).not.toHaveBeenCalled();
});

it.each([false, true])('releases capture and busy controls when final child cleanup rejects (stale=%s)', async stale => {
  const f = fixture((report, ordinal) => stale && ordinal === 2 ? { ...report, hull_content_id: 'stale' } : report);
  f.session.stop.mockResolvedValueOnce(undefined).mockResolvedValueOnce(undefined).mockRejectedValueOnce(new Error('bridge lost'));
  await expect(f.owner.start(defaultFittingPlan(f.ctx), f.ctx, 'sustained-engagement', 7))
    .rejects.toThrow(stale ? 'stale' : 'bridge lost');
  expect(f.owner.state().busy).toBe(false); expect(f.owner.state().results).toBeNull();
  expect(f.owner.state().error.message).toContain(stale ? 'stale' : 'bridge lost');
  // A later run can capture a new document rather than retaining the failed
  // run's source. The provider error does not strand the private UI capability.
  await f.owner.start(defaultFittingPlan(f.ctx), f.ctx, 'sustained-engagement', 7);
  expect(f.owner.state().results).not.toBeNull(); expect(f.capture).toHaveBeenCalledTimes(2);
});
