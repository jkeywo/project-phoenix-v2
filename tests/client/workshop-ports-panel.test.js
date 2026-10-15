/** @vitest-environment jsdom */
import fs from 'node:fs';
import { describe, expect, it, vi } from 'vitest';
import { parse, stringify } from 'smol-toml';
import { WorkshopDocument } from '../../editor/workshop-document.js';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { resolveTemplate } from '../../editor/entity-includes.js';
import { mountWorkshopPorts } from '../../gui/workshop-ports-panel.js';
const path = 'assets/entities/test.toml';
const control = id => document.getElementById('workshop-ports-' + id);
const settle = async () => { for (let i = 0; i < 25; i++) await new Promise(resolve => setTimeout(resolve, 1)); };
function fixture() {
  document.body.innerHTML = '<main></main>';
  const value = resolveTemplate('assets/entities/station_axiom.toml', { read: file => fs.existsSync(file) ? fs.readFileSync(file, 'utf8') : null }, parse).resolved.value;
  const source = stringify(value), draft = new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: '[pack]\nformat=1\nid="test"\nname="Test"\nversion="1"\n' }, { path, text: source }]));
  const runtime = { dependencies: async () => ({ base_files: {}, packs: [] }), validate: vi.fn(async () => ({ accepted: true })) };
  const panel = mountWorkshopPorts({ root: document.querySelector('main'), runtime, draft: () => draft, busy: () => false, setBusy: vi.fn() });
  return { draft, runtime, panel, source };
}
describe('Workshop port permission and lot terms', () => {
  it('keeps the actual producer binding through edits and explicitly chooses legacy through accepted history', async () => {
    const { draft, runtime, panel, source } = fixture(); await settle();
    expect(control('producer').value).toBe('dock');
    const select = control('producer'); select.focus(); panel.refresh(); expect(control('producer')).toBe(select); expect(document.activeElement).toBe(select);
    control('quota-0').value = '2'; control('apply').click(); await settle();
    expect(parse(draft.read(path)).port_service.system).toBe('dock');
    control('producer').value = ''; control('apply').click(); await settle();
    expect(parse(draft.read(path)).port_service.system).toBeUndefined(); expect(runtime.validate).toHaveBeenCalledTimes(2);
    draft.undo(); panel.refresh(); expect(control('producer').value).toBe('dock'); draft.undo(); expect(draft.read(path)).toBe(source); panel.dispose();
  });
  it('saves explicit quotas and deny-all membership through accepted source/history', async () => {
    const { draft, runtime, panel, source } = fixture(); await settle();
    for (const input of panel.node.querySelectorAll('input,select')) expect(panel.node.querySelector(`label[for="${input.id}"]`)).not.toBeNull();
    control('quota-0').value = '3'; control('factions').value = ''; control('apply').click(); await settle();
    expect(parse(draft.read(path)).port_service.offer[0].quota).toBe(3); expect(parse(draft.read(path)).port_service.eligible_factions).toEqual([]);
    expect(runtime.validate).toHaveBeenCalledTimes(1); draft.undo(); expect(draft.read(path)).toBe(source); panel.dispose();
  });
  it('preserves the exact draft/history on backend refusal and keeps bounded exchange controls editable', async () => {
    const { draft, runtime, panel, source } = fixture(); await settle(); runtime.validate.mockResolvedValueOnce({ accepted: false, findings: [{ message: 'provider refused' }] });
    control('policy').value = 'manual_offer'; control('quota-0').value = '2'; control('apply').click(); await settle();
    expect(control('status').textContent).toContain('provider refused'); expect(draft.read(path)).toBe(source); expect(draft.snapshot().history.undo).toHaveLength(0); panel.dispose();
  });
});
