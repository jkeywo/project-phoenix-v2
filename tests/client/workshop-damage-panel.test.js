/** @vitest-environment jsdom */
import { describe, it, expect, vi } from 'vitest';
import { parse, stringify } from 'smol-toml';
import { WorkshopDocument } from '../../editor/workshop-document.js';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { damageFixture } from '../../editor/tests/damage-fixture.js';
import { mountWorkshopDamage } from '../../gui/workshop-damage-panel.js';
const path = 'assets/entities/ship.toml', base = 'assets/entities/base.toml';
const control = id => document.getElementById('workshop-damage-' + id);
const select = (id, value) => { control(id).value = value; control(id).dispatchEvent(new Event('change')); };
async function settle() { for (let i = 0; i < 25; i++) await new Promise(resolve => setTimeout(resolve, 1)); }
function fixture(source = damageFixture().source, dependencies = { base_files: {}, packs: [] }) {
  document.body.innerHTML = '<main></main>';
  const draft = new WorkshopDocument(createStoreZip([
    { path: 'scenarios.toml', text: '[pack]\nformat=1\nid="mine"\nname="Mine"\nversion="1"\n' }, { path, text: source }]));
  const runtime = { shipSchema: async () => ({ damage: { bindings: ['operation', 'intake', 'cooling'], outcomes: ['structural', 'full', 'patch'], damage_types: ['physical', 'laser'] } }),
    dependencies: async () => dependencies, validate: vi.fn(async () => ({ accepted: true, findings: [] })) };
  const changed = vi.fn(), panel = mountWorkshopDamage({ root: document.querySelector('main'), runtime, draft: () => draft,
    busy: () => false, setBusy: vi.fn(), changed });
  return { draft, runtime, changed, panel, source };
}
describe('Workshop damage and restoration forms', () => {
  it('labels controls and applies physical material/recipe edits through accepted history', async () => {
    const { draft, runtime, panel, source } = fixture(); await settle();
    for (const input of panel.node.querySelectorAll('input,select')) expect(panel.node.querySelector(`label[for="${input.id}"]`)).not.toBeNull();
    select('materials', 'armour'); control('resistance-physical').value = '3'; control('save-material').click(); await settle();
    expect(parse(draft.read(path)).damage.materials.armour.resistance.physical).toBe(3);
    select('remedies', 'structural'); control('manual-work-fraction').value = '0.4'; control('save-remedy').click(); await settle();
    expect(parse(draft.read(path)).damage.remedies.structural.manual_work_fraction).toBe(0.4);
    expect(runtime.validate).toHaveBeenCalledTimes(2); draft.undo(); draft.undo(); panel.refresh(); expect(draft.read(path)).toBe(source); panel.dispose();
  });
  it('edits a generated arc in its canonical row and retains controller and local energy', async () => {
    const { draft, panel } = fixture(); await settle(); const before = parse(draft.read(path));
    select('systems', 'shield-arc-fore'); control('threshold').value = '30'; control('save-component').click(); await settle();
    const after = parse(draft.read(path)); expect(after.shield_arc[0].damage_component[0].fault_dose_threshold).toBe(30);
    expect(after.shield_arc[0].ai).toEqual(before.shield_arc[0].ai); expect(after.energy).toEqual(before.energy);
    expect(after.system.some(row => row.id === 'shield-arc-fore')).toBe(false); panel.dispose();
  });
  it('keeps source/history on invalid manual work or authoritative refusal', async () => {
    const { draft, runtime, panel, source } = fixture(); await settle();
    control('manual-work-fraction').value = '0'; control('save-remedy').click(); await settle();
    expect(draft.read(path)).toBe(source); expect(runtime.validate).not.toHaveBeenCalled();
    control('manual-work-fraction').value = '0.25';
    runtime.validate.mockResolvedValueOnce({ accepted: false, findings: [{ message: 'part service unavailable' }] });
    control('save-remedy').click(); await settle(); expect(control('status').textContent).toContain('part service unavailable');
    expect(draft.read(path)).toBe(source); expect(draft.snapshot().history.undo).toHaveLength(0); panel.dispose();
  });
  it('requires explicit inherited materialisation and preserves the dependency', async () => {
    const inherited = damageFixture().source, source = 'includes=["base.toml"]\n# child\n';
    const dependencies = { base_files: { [base]: inherited }, packs: [] };
    const { draft, panel } = fixture(source, dependencies); await settle();
    control('residual').value = '0.2'; control('save-material').click(); await settle(); expect(draft.read(path)).toBe(source);
    control('materialise').checked = true; control('save-material').click(); await settle();
    expect(parse(draft.read(path)).damage.materials.light.residual_fraction).toBe(0.2); expect(dependencies.base_files[base]).toBe(inherited); panel.dispose();
  });
  it('creates an explicit profile with finite actual parts and no legacy damage buckets', async () => {
    const value = damageFixture().value; delete value.damage;
    for (const system of value.system) delete system.damage_component;
    for (const arc of value.shield_arc) delete arc.damage_component;
    for (const id of ['structural_parts', 'machinery_parts']) { delete value.resource_services[id]; value.infrastructure.capacity = value.infrastructure.capacity.filter(row => row.id !== id); }
    const { draft, panel } = fixture(stringify(value)); await settle(); control('create').click(); await settle();
    const after = parse(draft.read(path)); expect(after.damage.remedies.structural.manual_work_fraction).toBe(0.25);
    expect(after.infrastructure.capacity.find(row => row.id === 'machinery_parts').amount).toBeGreaterThan(0);
    expect(after.resource_services.structural_parts).toEqual({ kind: 'capacity', id: 'structural_parts' });
    expect(after.system.every(row => row.damage_component.length)).toBe(true); expect(after.hull).toBeUndefined(); panel.dispose();
  });
});
