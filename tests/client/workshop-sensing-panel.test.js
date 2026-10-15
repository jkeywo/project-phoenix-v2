/** @vitest-environment jsdom */
import { describe, expect, it, vi } from 'vitest';
import { parse, stringify } from 'smol-toml';
import { damageFixture } from '../../editor/tests/damage-fixture.js';
import { WorkshopDocument } from '../../editor/workshop-document.js';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { mountWorkshopSensing } from '../../gui/workshop-sensing-panel.js';
const path = 'assets/entities/test.toml', control = id => document.getElementById('workshop-sensing-' + id);
const settle = async () => { for (let i = 0; i < 30; i++) await new Promise(resolve => setTimeout(resolve, 1)); };
function fixture(withoutPassive = false) {
  document.body.innerHTML = '<main></main>'; const { value, schema } = damageFixture(); schema.console_instance_publication_kinds.passive_receiver = 'Passive';
  if (withoutPassive) {
    const id = value.sensing.passive.system; delete value.sensing.passive; value.system = value.system.filter(row => row.id !== id); delete value.interior.placements[id];
    value.energy.consumer = value.energy.consumer.filter(row => row.system !== id);
    for (const console of value.console) console.feed = console.feed?.filter(row => row.source !== id);
    for (const station of value.station) for (const rating of station.rating || []) for (const key of ['automated_systems', 'detailed_systems']) if (rating[key]) rating[key] = rating[key].filter(other => other !== id);
  }
  const source = stringify(value);
  const draft = new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: '[pack]\nformat=1\nid="test"\nname="Test"\nversion="1"\n' }, { path, text: source }]));
  const runtime = { dependencies: async () => ({ base_files: {}, packs: [] }), shipSchema: async () => schema, validate: vi.fn(async () => ({ accepted: true })) };
  let busy = false; const panel = mountWorkshopSensing({ root: document.querySelector('main'), runtime, draft: () => draft, busy: () => busy, setBusy: value => { busy = value; } });
  return { draft, runtime, panel, source };
}
describe('Workshop structured sensing controls', () => {
  it('labels every control and edits timed work and passive mount through accepted history', async () => {
    const { draft, panel, source } = fixture(); await settle();
    for (const input of panel.node.querySelectorAll('input,select')) expect(panel.node.querySelector(`label[for="${input.id}"]`)).not.toBeNull();
    control('focused-work_secs').value = '5'; control('passive-arc_deg').value = '90'; control('save').click(); await settle();
    expect(parse(draft.read(path)).sensing.focused.work_secs).toBe(5); expect(parse(draft.read(path)).sensing.passive.mount.arc_deg).toBe(90);
    draft.undo(); expect(draft.read(path)).toBe(source); panel.dispose();
  });
  it('offers only actual role hardware and preserves source after authoritative refusal', async () => {
    const { draft, panel, runtime, source } = fixture(); await settle();
    expect([...control('passive-system').options].map(option => option.value)).toEqual(['', 'passive-receiver']);
    expect([...control('room').options].map(option => option.value)).toContain('bridge');
    runtime.validate.mockResolvedValueOnce({ accepted: false, findings: [{ message: 'mount refused' }] });
    control('passive-range').value = '200'; control('save').click(); await settle();
    expect(control('status').textContent).toContain('mount refused'); expect(draft.read(path)).toBe(source); expect(draft.snapshot().history.undo).toHaveLength(0); panel.dispose();
  });
  it('retains unsaved field edits on unrelated refresh and disables the form during acceptance', async () => {
    const { panel, runtime } = fixture(); await settle(); control('focused-work_secs').value = '6'; panel.refresh();
    expect(control('focused-work_secs').value).toBe('6');
    let accept; runtime.validate.mockImplementationOnce(() => new Promise(resolve => { accept = resolve; })); control('save').click(); await settle();
    expect(control('save').disabled).toBe(true); expect(control('focused-work_secs').disabled).toBe(true);
    accept({ accepted: true }); await settle(); expect(control('save').disabled).toBe(false); panel.dispose();
  });
  it('fits the exact edited passive cadence and mount rather than replacing them with defaults', async () => {
    const { draft, panel, runtime } = fixture(true); await settle();
    control('console').value = 'science'; control('group').value = 'energy-passive-receiver'; control('room').value = 'bridge';
    control('passive-range').value = '240'; control('passive-sample_interval_secs').value = '2'; control('passive-arc_deg').value = '120';
    control('fit').click(); await settle();
    expect(runtime.validate).toHaveBeenCalledOnce(); expect(parse(draft.read(path)).sensing.passive).toMatchObject({ system: 'passive-receiver', sample_interval_secs: 2, mount: { range: 240, arc_deg: 120 } });
    expect(parse(draft.read(path)).energy.consumer.find(row => row.system === 'passive-receiver')).toBeTruthy(); panel.dispose();
  });
});
