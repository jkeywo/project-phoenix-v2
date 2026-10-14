/** @vitest-environment jsdom */
import { describe, it, expect, vi } from 'vitest';
import { parse } from 'smol-toml';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { WorkshopDocument } from '../../editor/workshop-document.js';
import { damageFixture } from '../../editor/tests/damage-fixture.js';
import { mountWorkshopWeapons } from '../../gui/workshop-weapon-panel.js';
const path = 'assets/entities/ship.toml';
const source = '# root\n[[weapon]]\nid="beam-a"\n[weapon.impact]\ndamage_type="laser"\npenetration=20\n[weapon.mount]\nfacing_deg=0\nfire_arc_deg=90\nrange=40\n[weapon.delivery]\nkind="beam"\nauto_arc_deg=90\ndamage_per_sec=10\nduration_secs=1\ncooldown_secs=1\n[weapon.ai]\nidle=true\n';
async function settle() { for (let i = 0; i < 20; i++) await new Promise(resolve => setTimeout(resolve, 1)); }
function fixture(text = source, schema = {}) {
  document.body.innerHTML = '<main></main>';
  const current = new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: '[pack]\nformat=1\nid="mine"\nname="Mine"\nversion="1"\n' }, { path, text }]));
  const runtime = { shipSchema: async () => schema, dependencies: async () => ({ base_files: {}, packs: [] }), validate: vi.fn(async () => ({ accepted: true, findings: [] })) };
  const changed = vi.fn(), panel = mountWorkshopWeapons({ root: document.querySelector('main'), runtime, draft: () => current, busy: () => false, setBusy: vi.fn(), changed });
  return { current, runtime, changed, panel };
}
const control = id => document.getElementById('workshop-weapon-' + id);
describe('Workshop configurable weapon form', () => {
  it('edits a real source through labelled controls and shared history', async () => {
    const { current, runtime, changed, panel } = fixture(); await settle();
    for (const input of panel.node.querySelectorAll('input,select')) expect(panel.node.querySelector(`label[for="${input.id}"]`)).not.toBeNull();
    control('range').value = '60'; control('save').click(); await settle();
    expect(parse(current.read(path)).weapon[0]).toMatchObject({ mount: { range: 60 }, ai: { idle: true } });
    expect(changed).toHaveBeenCalledWith(path); expect(runtime.validate).toHaveBeenCalledTimes(1);
    current.undo(); panel.refresh(); expect(current.read(path)).toBe(source); panel.dispose();
  });
  it('creates a supported bolt with editable barrel steps and refuses unsupported candidates', async () => {
    const hardware = damageFixture();
    const { current, runtime, panel } = fixture(hardware.source, hardware.schema); await settle(); control('new').click();
    control('kind').value = 'bolt'; control('kind').dispatchEvent(new Event('change'));
    control('id').value = 'bolt-b'; control('strategy-id').value = 'b'; control('barrels').value = 'left, right';
    control('console').value = hardware.value.console[0].id; control('group').value = hardware.value.energy.group[0].id; control('compartment').value = hardware.value.interior.compartment[0].id;
    control('add-step').click(); control('step-barrels-0').value = '0, 1'; control('save').click(); await settle();
    expect(control('status').textContent).not.toContain('refused');
    expect(parse(current.read(path)).weapon.at(-1)).toMatchObject({ id: 'bolt-b', strategy_id: 'b', delivery: { kind: 'bolt' }, barrels: ['left', 'right'], pattern: [{ offset_secs: 0, barrels: [0, 1] }] });
    const before = current.read(path); runtime.validate.mockResolvedValueOnce({ accepted: false, findings: [{ message: 'barrel out of range' }] });
    control('step-barrels-1').value = '9'; control('save').click(); await settle();
    expect(current.read(path)).toBe(before); expect(control('status').textContent).toContain('barrel out of range'); panel.dispose();
  });
});
