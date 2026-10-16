// @vitest-environment jsdom
import { afterEach, expect, it, vi } from 'vitest';
import { mountWorkshopTestPanel } from '../../gui/workshop-test-panel.js';
import { mountFittingPlans } from '../../gui/components/fitting-plan-controls.js';
import { rehearsalChoices, REHEARSAL_WORLDS } from '../../editor/fitting-rehearsal.js';
import { WorkshopDocument } from '../../editor/workshop-document.js';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { WORKSHOP_MANIFEST, WORKSHOP_WORLD, WORKSHOP_WORLD_TEXT } from '../fixtures/workshop-pack.js';

let panel;
afterEach(() => { panel?.dispose(); panel = null; document.body.replaceChildren(); sessionStorage.clear(); });
const context = () => ({ hull_path: 'assets/entities/alliance_cruiser.toml', content_id: '0000000000000013', slots: [
  { slot: 'fore-beam', system: 'phaser-fore', home_station: 'tactical', default_equipment: 'beam-standard', compatible_equipment: ['beam-standard', 'beam-pulse'] },
] });
it('offers the actual private Workshop favourite action through normal captured Test start without editing the document', async () => {
  const ctx = context(), captured = [];
  const draft = new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: WORKSHOP_MANIFEST },
    { path: WORKSHOP_WORLD, text: WORKSHOP_WORLD_TEXT }]));
  const before = draft.sourceRevision;
  const start = vi.fn(async (files, selection) => {
    captured.push(structuredClone(files));
    const report = { revision: 'captured', hull_content_id: ctx.content_id, workload_revision: 'reference',
      run_generation: selection.run_generation, selection: { world: selection.world, ship: selection.ship, slot: selection.slot, seed: selection.seed },
      choices: rehearsalChoices(selection.rehearsal.plan, ctx), profile: { workload: selection.rehearsal.workload, phases: [] },
      complete: true, measured_intervals: 60, fixed_period_secs: 1 / 60, generated_energy: 4,
      initial_coolant: { coolant: 10 }, final_coolant: { coolant: 9.5 }, flow_cut_intervals: 0,
      engagement_intervals: 60, achieved_engagement_intervals: 3, subject_structural_hp_lost: 1,
      buffers: { 'phaser-fore': { initial_energy: 5, final_energy: 4, minimum_energy: 4, maximum_heat: 2, paid_energy: 4, thermal_trips: 0, thermal_recoveries: 0 } },
      charts: [{ measured_intervals: 60, reserve: 4 }], authored_initial_power: {}, nominal_equipment: {},
    };
    return { running: true, revision: 'captured', paused: true, tick: 60, rehearsal: report };
  });
  panel = mountWorkshopTestPanel({ root: document.body, draft: () => draft, busy: () => false,
    provider: { test: { catalog: async () => ({ worlds: Object.values(REHEARSAL_WORLDS), ships: [ctx.hull_path], rehearsal_fitting: ctx }),
      capture: current => current.toNativeSources(), start, stop: async () => {}, status: async () => null } } });
  const action = () => document.querySelector('[data-plan-action="rehearse"]');
  await vi.waitFor(() => expect(action()?.disabled).toBe(false));
  action().click();
  await vi.waitFor(() => expect(start).toHaveBeenCalledTimes(2));
  await vi.waitFor(() => expect(document.querySelectorAll('#workshop-test-rehearsal-report details')).toHaveLength(2));
  expect(captured[0]).toEqual(captured[1]);
  expect(draft.sourceRevision).toBe(before);
  for (const [, selection] of start.mock.calls) {
    expect(selection.slot).toBe('rehearsal');
    expect(Object.keys(selection.rehearsal).sort()).toEqual(['plan', 'version', 'workload']);
    expect(selection).not.toHaveProperty('system_id');
  }
});
it('does not add private Test capability to ordinary phone or lobby fitting controls', () => {
  const ordinary = mountFittingPlans(document, { prelaunch: true }); document.body.append(ordinary.node);
  ordinary.update([{ fitting_planner: context() }]);
  expect(ordinary.node.querySelector('[data-plan-action="rehearse"]')).toBeNull();
  ordinary.dispose();
});
