/** @vitest-environment jsdom */
import { describe, expect, it, vi } from 'vitest';
import { parse } from 'smol-toml';
import { WorkshopDocument } from '../../editor/workshop-document.js';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { mountWorkshopWeaponGroups } from '../../gui/workshop-weapon-groups-panel.js';
const path = 'assets/entities/test.toml', source = '[[system]]\nid="beam"\nkind="phaser_bank"\n[weapons_console]\nmode="Manual"\n';
const control = id => document.getElementById('workshop-weapon-groups-' + id);
const settle = async () => { for (let i = 0; i < 15; i++) await new Promise(resolve => setTimeout(resolve, 1)); };
const documentFor = text => new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: '[pack]\nformat=1\nid="test"\nname="Test"\nversion="1"\n' }, { path, text }]));
function fixture() {
  document.body.innerHTML = '<main></main>';
  const draft = documentFor(source); let currentDraft = draft;
  const runtime = { dependencies: async () => ({ base_files: {}, packs: [] }), validate: vi.fn(async () => ({ accepted: true })) }, preview = vi.fn(), test = vi.fn();
  let busy = false;
  const panel = mountWorkshopWeaponGroups({ root: document.querySelector('main'), runtime, draft: () => currentDraft, busy: () => busy, setBusy: value => { busy = value; }, preview, test });
  return { draft, panel, runtime, preview, test, replaceDraft: value => { currentDraft = value; } };
}
describe('Workshop prepared group form', () => {
  it('selects actual equipment, accepts edits/history and clears the final group', async () => {
    const { draft, panel, runtime } = fixture(); await settle();
    control('new').click(); control('id').value = 'beams'; control('label').value = 'weapon.group.main_beams'; control('members').querySelector('input').checked = true;
    control('save').click(); await settle(); expect(runtime.validate).toHaveBeenCalledOnce(); expect(parse(draft.read(path)).weapons_console.group[0].members).toEqual(['beam']);
    draft.undo(); panel.refresh(); expect(draft.read(path)).toBe(source); draft.redo(); panel.refresh(); control('remove').click(); await settle();
    expect(parse(draft.read(path)).weapons_console.group).toEqual([]); panel.dispose();
  });
  it('retains unsaved focused input on refresh and uses existing Preview/Test callbacks', async () => {
    const { panel, preview, test } = fixture(); await settle(); const input = control('id'); input.value = 'draft'; input.focus(); panel.refresh();
    expect(document.activeElement).toBe(input); expect(input.value).toBe('draft'); control('preview').click(); control('test').click();
    expect(preview).toHaveBeenCalledWith(path); expect(test).toHaveBeenCalledWith(path); panel.dispose();
  });
  it('does not commit refused candidates or history', async () => {
    const { panel, runtime, draft } = fixture(); await settle(); control('id').value = 'pair'; control('label').value = 'weapon.group.main_beams'; control('members').querySelector('input').checked = true;
    runtime.validate.mockResolvedValueOnce({ accepted: false, findings: [{ message: 'actual hardware refused' }] }); control('save').click(); await settle();
    expect(draft.read(path)).toBe(source); expect(draft.snapshot().history.undo).toHaveLength(0); expect(control('status').textContent).toContain('actual hardware refused'); panel.dispose();
  });
  it('refreshes a replacement document with the same path and revision', async () => {
    const { panel, replaceDraft } = fixture(); await settle();
    replaceDraft(documentFor(source.replace('id="beam"', 'id="other-bank"'))); panel.refresh();
    expect(control('members').querySelector('input').value).toBe('other-bank'); panel.dispose();
  });
});
