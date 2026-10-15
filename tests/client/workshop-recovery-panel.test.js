/** @vitest-environment jsdom */
import { describe, expect, it, vi } from 'vitest';
import { parse } from 'smol-toml';
import { damageFixture } from '../../editor/tests/damage-fixture.js';
import { WorkshopDocument } from '../../editor/workshop-document.js';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { mountWorkshopRecovery } from '../../gui/workshop-recovery-panel.js';
const path = 'assets/entities/test.toml';
const control = id => document.getElementById('workshop-resource-recovery-' + id);
const settle = async () => { for (let i = 0; i < 25; i++) await new Promise(resolve => setTimeout(resolve, 1)); };
function fixture() {
  document.body.innerHTML = '<main></main>'; const { source, schema } = damageFixture(); schema.console_stock_publication_kind = 'Stock';
  const draft = new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: '[pack]\nformat=1\nid="test"\nname="Test"\nversion="1"\n' }, { path, text: source }]));
  const runtime = { dependencies: async () => ({ base_files: {}, packs: [] }), shipSchema: async () => schema, validate: vi.fn(async () => ({ accepted: true })) };
  const panel = mountWorkshopRecovery({ root: document.querySelector('main'), runtime, draft: () => draft, busy: () => false, setBusy: vi.fn() });
  return { draft, runtime, panel, source };
}
describe('Workshop recovery tool and public source manifest', () => {
  it('edits the closed actual tool through accepted source/history/export', async () => {
    const { draft, runtime, panel, source } = fixture(); await settle();
    for (const input of panel.node.querySelectorAll('input,select')) expect(panel.node.querySelector(`label[for="${input.id}"]`)).not.toBeNull();
    control('range').value = '9'; control('save-tool').click(); await settle();
    expect(parse(draft.read(path)).recovery.range).toBe(9); expect(runtime.validate).toHaveBeenCalledTimes(1);
    draft.undo(); expect(draft.read(path)).toBe(source); panel.dispose();
  });
  it('authors only references to actual source stocks and preserves source on authoritative refusal', async () => {
    const { draft, runtime, panel, source } = fixture(); await settle();
    control('add-resource').click(); control('resource-0').value = 'scrap'; control('endpoint-0').value = 'capacity:scrap'; control('save-source').click(); await settle();
    expect(parse(draft.read(path)).recoverable).toEqual({ availability: 'always', resources: { scrap: { kind: 'capacity', id: 'scrap' } } });
    draft.undo(); panel.refresh(); await settle();
    runtime.validate.mockResolvedValueOnce({ accepted: false, findings: [{ message: 'source refused' }] });
    control('range').value = '10'; control('save-tool').click(); await settle();
    expect(draft.read(path)).toBe(source); expect(control('status').textContent).toContain('source refused');
    expect(draft.snapshot().history.undo).toHaveLength(0); panel.dispose();
  });
});
