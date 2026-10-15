/** @vitest-environment jsdom */
import { describe, expect, it, vi } from 'vitest';
import { parse, stringify } from 'smol-toml';
import { fittingFixture } from '../../editor/tests/fitting-fixture.js';
import { WorkshopDocument } from '../../editor/workshop-document.js';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { mountWorkshopFitting } from '../../gui/workshop-fitting-panel.js';
const path = 'assets/entities/test.toml';
const settle = async () => { for (let i = 0; i < 25; i++) await new Promise(resolve => setTimeout(resolve, 1)); };
const control = key => document.querySelector(`[id^="workshop-fitting-${key}-"]`);
const click = key => document.querySelector(`[data-fitting-action="${key}"]`).click();
const change = (key, value) => { control(key).value = value; control(key).dispatchEvent(new Event('change')); };
function fixture({ empty = false } = {}) {
  document.body.innerHTML = '<main></main>'; const { value } = fittingFixture(); if (empty) delete value.fitting;
  const source = stringify(value), draft = new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: '[pack]\nformat=1\nid="test"\nname="Test"\nversion="1"\n' }, { path, text: source }]));
  const runtime = { dependencies: async () => ({ base_files: {}, packs: [] }), validate: vi.fn(async () => ({ accepted: true })) };
  const panel = mountWorkshopFitting({ root: document.querySelector('main'), runtime, draft: () => draft, busy: () => false, setBusy: vi.fn() });
  return { source, draft, runtime, panel };
}
describe('Workshop physical equipment form', () => {
  it('authors and preserves an optional payload when changing unrelated sidegrade work', async () => {
    const { panel, draft } = fixture(); await settle(); change('definitions', 'efficient');
    control('emp-enabled').checked = true; control('emp-enabled').dispatchEvent(new Event('change')); control('emp-duration').value = '7';
    click('save-definition'); await settle();
    expect(parse(draft.read(path)).fitting.equipment.find(row => row.id === 'efficient').hardware.impact.emp).toEqual({ duration_secs: 7 });
    control('work-secs').value = '13'; click('save-definition'); await settle();
    expect(parse(draft.read(path)).fitting.equipment.find(row => row.id === 'efficient').hardware.impact.emp).toEqual({ duration_secs: 7 });
    panel.dispose();
  });
  it('creates the first slot, equivalent default and one installed assembly atomically', async () => {
    const { panel, draft, runtime, source } = fixture({ empty: true }); await settle();
    for (const input of panel.node.querySelectorAll('input,select')) expect(panel.node.querySelector(`label[for="${input.id}"]`)).not.toBeNull();
    control('slot-id').value = 'main-beam'; control('default').value = 'beam-standard'; click('save-slot'); await settle();
    const result = parse(draft.read(path)).fitting;
    expect(result.slot).toHaveLength(1); expect(result.equipment).toHaveLength(1); expect(result.assembly).toEqual([{ id: 'main-beam-installed', definition: 'beam-standard', location: { kind: 'installed', slot: 'main-beam' } }]);
    expect(runtime.validate).toHaveBeenCalledTimes(1); draft.undo(); expect(draft.read(path)).toBe(source); panel.dispose();
  });
  it('edits supported hardware with exact history and keeps refused local form values', async () => {
    const { panel, draft, runtime, source } = fixture(); await settle(); change('definitions', 'efficient');
    control('work-secs').value = '12'; click('save-definition'); await settle();
    expect(parse(draft.read(path)).fitting.equipment.find(row => row.id === 'efficient').work.work_secs).toBe(12);
    draft.undo(); expect(draft.read(path)).toBe(source); panel.refresh(); await settle(); change('definitions', 'efficient');
    runtime.validate.mockResolvedValueOnce({ accepted: false, findings: [{ message: 'unsafe fitted state' }] });
    control('work-secs').value = '11'; click('save-definition'); await settle();
    expect(panel.node.querySelector('[role=status]').textContent).toContain('unsafe fitted state'); expect(control('work-secs').value).toBe('11');
    expect(draft.read(path)).toBe(source); panel.dispose();
  });
  it('keeps installed condition with Damage and refuses overfull stored equipment', async () => {
    const { panel, draft, source } = fixture(); await settle(); change('assemblies', 'installed');
    expect(control('condition').disabled).toBe(true); expect(document.querySelector('[data-fitting-action=save-assembly]').disabled).toBe(true);
    change('assemblies', ''); control('assembly-id').value = 'extra-spare'; control('assembly-definition').value = 'efficient'; control('assembly-locker').value = 'spares'; click('save-assembly'); await settle();
    expect(panel.node.querySelector('[role=status]').textContent).toContain('capacity'); expect(draft.read(path)).toBe(source); panel.dispose();
  });
});

it('round trips portable explicit empty targets and legacy false without numeric coercion', async () => {
  const {panel,draft}=fixture();await settle();change('definitions','efficient');change('targets-mode','explicit');click('save-definition');await settle();
  expect(parse(draft.read(path)).fitting.equipment.find(row=>row.id==='efficient').hardware.targets).toEqual([]);
  for(const option of control('target-tags').options)option.selected=option.value==='missile';click('save-definition');await settle();expect(parse(draft.read(path)).fitting.equipment.find(row=>row.id==='efficient').hardware.targets).toEqual(['missile']);
  change('targets-mode','legacy');click('save-definition');await settle();expect(parse(draft.read(path)).fitting.equipment.find(row=>row.id==='efficient').hardware.targets).toBe(false);panel.dispose();
});
