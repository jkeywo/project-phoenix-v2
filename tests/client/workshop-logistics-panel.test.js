/** @vitest-environment jsdom */
import { describe, it, expect, vi } from 'vitest';
import { parse } from 'smol-toml';
import { WorkshopDocument } from '../../editor/workshop-document.js';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { logisticsFixture } from '../../editor/tests/logistics-fixture.js';
import { mountWorkshopLogistics } from '../../gui/workshop-logistics-panel.js';
const path = 'assets/entities/test.toml', base = 'assets/entities/base.toml';
const control = id => document.getElementById('workshop-logistics-' + id);
const select = (id, value) => { control(id).value = value; control(id).dispatchEvent(new Event('change')); };
const settle = async () => { for (let i = 0; i < 25; i++) await new Promise(resolve => setTimeout(resolve, 1)); };
function fixture(source = logisticsFixture().source, dependencies = { base_files: {}, packs: [] }) {
  document.body.innerHTML = '<main></main>';
  const draft = new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: '[pack]\nformat=1\nid="test"\nname="Test"\nversion="1"\n' }, { path, text: source }]));
  const runtime = { dependencies: async () => dependencies, validate: vi.fn(async () => ({ accepted: true, findings: [] })) };
  const panel = mountWorkshopLogistics({ root: document.querySelector('main'), runtime, draft: () => draft, busy: () => false, setBusy: vi.fn() });
  return { draft, runtime, panel };
}
describe('Workshop located logistics controls', () => {
  it('labels typed controls and saves targets, storage and handling through accepted history', async () => {
    const { draft, runtime, panel } = fixture(); await settle();
    for (const input of panel.node.querySelectorAll('input,select')) expect(panel.node.querySelector(`label[for="${input.id}"]`)).not.toBeNull();
    control('reserve').value = '2'; control('save-store').click(); await settle();
    expect(parse(draft.read(path)).logistics.store.find(row => row.id === 'hold').reserve).toBe(2);
    control('space').value = '8'; control('save-storage').click(); await settle();
    expect(parse(draft.read(path)).logistics.storage.find(row => row.compartment === 'hold').space).toBe(8);
    control('max-batch').value = '3'; control('save-handling').click(); await settle();
    expect(parse(draft.read(path)).logistics.handling[0].max_batch).toBe(3); expect(runtime.validate).toHaveBeenCalledTimes(3);
    draft.undo(); draft.undo(); draft.undo(); expect(draft.read(path)).toBe(logisticsFixture().source); panel.dispose();
  });
  it('edits local targets in authored data without inventing extra capacities', async () => {
    const { draft, panel } = fixture(); await settle(); select('stores', 'local'); control('target').value = '5'; control('save-store').click(); await settle();
    const value = parse(draft.read(path)); expect(value.logistics.store.find(row => row.id === 'local').target).toBe(5);
    expect(value.infrastructure.capacity).toHaveLength(1); expect(value.torpedoes).toEqual({ count: 6, initial_count: 2 }); panel.dispose();
  });
  it('preserves source/history on capacity failure and authoritative route refusal', async () => {
    const { draft, runtime, panel } = fixture(); await settle(); const before = draft.read(path);
    control('space').value = '3'; control('save-storage').click(); await settle(); expect(runtime.validate).not.toHaveBeenCalled();
    expect(draft.read(path)).toBe(before); control('space').value = '6';
    runtime.validate.mockResolvedValueOnce({ accepted: false, findings: [{ message: 'route refused' }] });
    control('save-storage').click(); await settle(); expect(control('status').textContent).toContain('route refused');
    expect(draft.snapshot().history.undo).toHaveLength(0); expect(draft.read(path)).toBe(before); panel.dispose();
  });
  it('requires explicit inherited materialisation and safely renames a source binding', async () => {
    const inherited = logisticsFixture().source, before = 'includes=["base.toml"]\n# child\n';
    const { draft, panel } = fixture(before, { base_files: { [base]: inherited }, packs: [] }); await settle();
    control('store-id').value = 'new-hold'; control('save-store').click(); await settle(); expect(draft.read(path)).toBe(before);
    control('materialise').checked = true; control('save-store').click(); await settle();
    expect(parse(draft.read(path)).logistics.store.find(row => row.id === 'local').source).toBe('new-hold'); panel.dispose();
  });
});
