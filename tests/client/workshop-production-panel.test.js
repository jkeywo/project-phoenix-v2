/** @vitest-environment jsdom */
import { describe, expect, it, vi } from 'vitest';
import { parse } from 'smol-toml';
import { damageFixture } from '../../editor/tests/damage-fixture.js';
import { WorkshopDocument } from '../../editor/workshop-document.js';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { mountWorkshopProduction } from '../../gui/workshop-production-panel.js';
const path = 'assets/entities/test.toml';
const control = id => document.getElementById('workshop-production-' + id);
const settle = async () => { for (let i = 0; i < 25; i++) await new Promise(resolve => setTimeout(resolve, 1)); };
function fixture() {
  document.body.innerHTML = '<main></main>'; const { source, schema } = damageFixture(); schema.console_stock_publication_kind = 'Stock';
  const draft = new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: '[pack]\nformat=1\nid="test"\nname="Test"\nversion="1"\n' }, { path, text: source }]));
  const runtime = { dependencies: async () => ({ base_files: {}, packs: [] }), shipSchema: async () => schema, validate: vi.fn(async () => ({ accepted: true })) };
  const panel = mountWorkshopProduction({ root: document.querySelector('main'), runtime, draft: () => draft, busy: () => false, setBusy: vi.fn() });
  return { draft, runtime, panel, source };
}
describe('Workshop finite production controls', () => {
  it('edits exact recipe work and initial physical-stock target with authoritative history/export', async () => {
    const { draft, runtime, panel, source } = fixture(); await settle();
    const recipes = parse(source).production.recipe;
    expect(recipes.length).toBeGreaterThan(1);
    for (const input of panel.node.querySelectorAll('input,select')) expect(panel.node.querySelector(`label[for="${input.id}"]`)).not.toBeNull();
    control('work-secs').value = '12'; control('save-recipe').click(); await settle();
    const edited = parse(draft.read(path)).production.recipe;
    expect(edited[0].work_secs).toBe(12);
    expect(edited.map(row => row.id)).toEqual(recipes.map(row => row.id));
    expect(edited.slice(1)).toEqual(recipes.slice(1));
    control('initial-target').value = '6'; control('save-initial').click(); await settle();
    expect(parse(draft.read(path)).production.initial_target).toBe(6); expect(runtime.validate).toHaveBeenCalledTimes(2);
    draft.undo(); draft.undo(); expect(draft.read(path)).toBe(source); panel.dispose();
  });
  it('offers only actual same-compartment stores and preserves draft on authoritative refusal', async () => {
    const { draft, runtime, panel, source } = fixture(); await settle();
    const storeSelect = panel.node.querySelector('select[id*="inputs-store"]');
    expect([...storeSelect.options].some(option => option.value === 'repair-machinery')).toBe(false);
    expect([...storeSelect.options].some(option => option.value === 'hold-alloy')).toBe(true);
    runtime.validate.mockResolvedValueOnce({ accepted: false, findings: [{ message: 'recipe denied' }] });
    control('work-secs').value = '11'; control('save-recipe').click(); await settle();
    expect(control('status').textContent).toContain('recipe denied'); expect(draft.read(path)).toBe(source); expect(draft.snapshot().history.undo).toHaveLength(0); panel.dispose();
  });
  it('preserves authored output order when choosing the primary target product', async () => {
    const { draft, panel, source } = fixture(); await settle(); control('add-output').click();
    const recipes = parse(source).production.recipe;
    const rows = [...control('outputs').querySelectorAll('div')];
    rows[1].querySelector('select').value = 'hold-alloy';
    [...rows[1].querySelectorAll('button')].find(button => button.textContent === 'Move output earlier').click();
    control('save-recipe').click(); await settle();
    const edited = parse(draft.read(path)).production.recipe;
    expect(edited[0].outputs.map(row => row.store)).toEqual(['hold-alloy', 'hold-ammunition']);
    expect(edited.map(row => row.id)).toEqual(recipes.map(row => row.id));
    expect(edited.slice(1)).toEqual(recipes.slice(1)); panel.dispose();
  });
});
