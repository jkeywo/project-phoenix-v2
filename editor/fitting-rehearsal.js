import { defaultFittingPlan, validateFittingPlan } from '../gui/fitting-plans.js';

export const REHEARSAL_WORLDS = Object.freeze({
  'sustained-engagement': 'assets/worlds/fitting_rehearsal_sustained.toml',
  'burst-recovery': 'assets/worlds/fitting_rehearsal_burst.toml',
});
export function rehearsalChoices(plan, context) {
  validateFittingPlan(plan, context);
  return Object.fromEntries(context.slots.map(slot => [slot.slot,
    plan.sections[slot.home_station]?.choices[slot.slot] ?? slot.default_equipment]));
}
const sameChoices = (a, b) => Object.keys(a).length === Object.keys(b || {}).length
  && Object.entries(a).every(([key, value]) => b[key] === value);

/** One comparison owns the existing disposable Test session serially. Both
 * runs receive defensive copies of one frozen document capture. Its immutable
 * dependency owner remains the ordinary provider; a changed revision refuses
 * comparison rather than quietly mixing contexts. */
export function createFittingRehearsal({ session, capture, useCapture, onChange = () => {},
  timer = globalThis, timeoutMs = 180000 }) {
  let generation = 0, busy = false, results = null, error = null, disposed = false;
  const state = () => ({ busy, results, error });
  const notify = () => { if (!disposed) onChange(state()); };
  function current(token) { if (disposed || token !== generation) throw new Error('fitting.rehearsal.cancelled'); }
  async function trial(token, selection, expected, context, workload, expectedRevision) {
    current(token); await session.start(selection); current(token);
    const began = Date.now();
    for (;;) {
      current(token);
      const run = session.state().run;
      if (!run || run.error) throw new Error(run?.error || 'fitting.rehearsal.failed');
      const report = run.rehearsal;
      if (report && (report.run_generation !== selection.run_generation
        || report.hull_content_id !== context.content_id || report.profile?.workload !== workload
        || report.selection?.world !== selection.world || report.selection?.ship !== selection.ship
        || report.selection?.slot !== selection.slot || report.selection?.seed !== selection.seed
        || !sameChoices(expected, report.choices)
        || report.revision !== run.revision || expectedRevision && report.revision !== expectedRevision)) {
        throw new Error('fitting.rehearsal.stale');
      }
      if (report?.error) throw new Error(report.error);
      if (report?.complete) return structuredClone(report);
      if (Date.now() - began > timeoutMs) throw new Error('fitting.rehearsal.timeout');
      await new Promise(resolve => timer.setTimeout(resolve, 100));
      current(token); await session.poll();
    }
  }
  return {
    state,
    async start(plan, context, workload, seed) {
      if (disposed || busy) throw new Error('fitting.rehearsal.busy');
      validateFittingPlan(plan, context);
      context = structuredClone(context);
      if (context.hull_path !== 'assets/entities/alliance_cruiser.toml' || !REHEARSAL_WORLDS[workload]
        || !Number.isSafeInteger(seed) || seed < 0) throw new Error('fitting.rehearsal.unsupported');
      const chosen = structuredClone(plan), baseline = defaultFittingPlan(context);
      const files = structuredClone(capture());
      const token = ++generation;
      if (!Number.isSafeInteger(token * 2)) throw new Error('fitting.rehearsal.failed');
      busy = true; results = null; error = null; notify();
      useCapture(files);
      try {
        await session.stop(); current(token);
        const selection = { world: REHEARSAL_WORLDS[workload], ship: context.hull_path, slot: 'rehearsal', seed };
        const make = (favourite, serial) => ({ ...selection, run_generation: token * 2 + serial,
          rehearsal: { version: 1, plan: favourite, workload } });
        const first = await trial(token, make(baseline, 0), rehearsalChoices(baseline, context), context, workload, null);
        await session.stop(); current(token);
        const second = await trial(token, make(chosen, 1), rehearsalChoices(chosen, context), context, workload, first.revision);
        if (first.workload_revision !== second.workload_revision || first.fixed_period_secs !== second.fixed_period_secs) {
          throw new Error('fitting.rehearsal.stale');
        }
        current(token); results = { baseline: first, selected: second };
      } catch (failure) {
        if (!disposed && token === generation) { error = failure; results = null; }
        throw failure;
      } finally {
        if (token === generation) {
          let cleanupError;
          try { await session.stop(); }
          catch (failure) {
            if (token === generation) {
              results = null;
              if (!error) { error = failure; cleanupError = failure; }
            }
          } finally { if (token === generation) { useCapture(null); busy = false; notify(); } }
          if (cleanupError && token === generation) throw cleanupError;
        }
      }
    },
    changed() { const wasBusy = busy; generation++; results = null; error = null; useCapture(null); busy = false;
      const token = generation;
      if (wasBusy) void session.stop().catch(failure => { if (!disposed && generation === token) { error = failure; notify(); } }); notify(); },
    async cancel() { const token = ++generation; results = null; error = null; useCapture(null); busy = false;
      try { await session.stop(); } catch (failure) { if (generation === token) error = failure; throw failure; }
      finally { if (generation === token) notify(); } },
    dispose() { const wasBusy = busy; disposed = true; generation++; useCapture(null); return wasBusy ? session.stop() : Promise.resolve(); },
  };
}
