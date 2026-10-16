/** @vitest-environment jsdom */
import { describe, it, expect, vi } from 'vitest';
import { parse } from 'smol-toml';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { WorkshopDocument } from '../../editor/workshop-document.js';
import { shieldSource } from '../../editor/tests/shield-fixture.js';
import { mountWorkshopShields } from '../../gui/workshop-shield-panel.js';
const path = 'assets/entities/ship.toml';
const control = id => document.getElementById('workshop-shield-' + id);
async function settle() { for (let i = 0; i < 15; i++) await new Promise(resolve => setTimeout(resolve, 1)); }
function fixture() {
  document.body.innerHTML = '<main></main>';
  const source = shieldSource(), draft = new WorkshopDocument(createStoreZip([
    { path: 'scenarios.toml', text: '[pack]\nformat=1\nid="mine"\nname="Mine"\nversion="1"\n' }, { path, text: source },
  ]));
  const runtime = { dependencies: async () => ({ base_files: {}, packs: [] }), validate: vi.fn(async () => ({ accepted: true, findings: [] })) };
  const changed = vi.fn(), panel = mountWorkshopShields({ root: document.querySelector('main'), runtime, draft: () => draft,
    busy: () => false, setBusy: vi.fn(), changed });
  return { source, draft, runtime, changed, panel };
}
describe('Workshop shield form', () => {
  it('authors optional response/cycling through the existing accepted history and retains an unsaved draft', async () => {
    const {draft,panel,runtime}=fixture();await settle();
    const frequency=control('frequencies');frequency.value='0.2, 0.8';frequency.focus();panel.refresh();
    expect(document.activeElement).toBe(frequency);expect(frequency.value).toBe('0.2, 0.8');
    control('mismatch-loss').value='0.6';control('cycling').checked=true;control('cycle-period').value='30';
    control('frequency-save').click();await settle();
    expect(parse(draft.read(path)).shields_console).toMatchObject({frequency_response:{mismatch_loss:.6},frequency_cycle:{frequencies:[.2,.8],period_secs:30}});
    expect(runtime.validate).toHaveBeenCalledOnce();draft.undo();panel.refresh();expect(parse(draft.read(path)).shields_console?.frequency_cycle).toBeUndefined();panel.dispose();
  });
  it('labels physical work and recovery controls and edits through ordinary history', async () => {
    const { source, draft, changed, panel } = fixture(); await settle();
    for (const input of panel.node.querySelectorAll('input,select')) expect(panel.node.querySelector(`label[for="${input.id}"]`)).not.toBeNull();
    control('upkeep_per_sec').value = '0.2';
    expect(control('focus-available').checked).toBe(true);
    control('focus-available').checked = false;
    [...control('targets').options].find(option => option.value === 'shield-arc-aft').selected = true;
    control('reset-ai').checked = true; control('save').click(); await settle();
    expect(parse(draft.read(path)).shield_arc[0].energy).toMatchObject({ upkeep_per_sec: 0.2, recovery_targets: ['shield-arc-aft'] });
    expect(parse(draft.read(path)).shield_arc[0].focus_available).toBe(false);
    expect(changed).toHaveBeenCalledWith(path); draft.undo(); panel.refresh(); expect(draft.read(path)).toBe(source); panel.dispose();
  });
  it('creates and removes a powered arc as complete accepted edits', async () => {
    const { draft, panel } = fixture(); await settle(); control('new').click();
    control('id').value = 'port_aux'; control('label').value = 'Port'; control('save').click(); await settle();
    expect(parse(draft.read(path)).energy.consumer.at(-1)).toMatchObject({ system: 'shield-arc-port-aux', initial_energy: 50 });
    expect([...control('targets').options].some(option => option.value === 'shield-arc-port-aux')).toBe(true);
    expect(control('id').disabled).toBe(true); control('remove').click(); await settle();
    expect(parse(draft.read(path)).shield_arc.map(arc => arc.id)).toEqual(['fore', 'aft']);
    expect(parse(draft.read(path)).energy.consumer.some(row => row.system === 'shield-arc-port-aux')).toBe(false);
    expect(parse(draft.read(path)).power_groups['energy-shield-arc-port-aux']).toBeUndefined();
    expect(draft.snapshot().history.undo).toHaveLength(2); panel.dispose();
  });
  it('shows authoritative refusal without committing the candidate', async () => {
    const { draft, source, runtime, panel } = fixture(); await settle();
    runtime.validate.mockResolvedValueOnce({ accepted: false, findings: [{ message: 'Shield tree uses an unavailable target' }] });
    control('upkeep_per_sec').value = '0.2'; control('save').click(); await settle();
    expect(draft.read(path)).toBe(source); expect(control('status').textContent).toContain('unavailable target'); panel.dispose();
  });
});
