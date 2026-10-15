/** @vitest-environment jsdom */
import { describe, expect, it, vi } from 'vitest';
import { parse } from 'smol-toml';
import { WorkshopDocument } from '../../editor/workshop-document.js';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { mountWorkshopHelmProjection } from '../../gui/workshop-helm-projection-panel.js';
const path = 'assets/entities/test.toml', control = id => document.getElementById('workshop-helm-projection-' + id);
const settle = async () => { for (let i = 0; i < 30; i++) await new Promise(resolve => setTimeout(resolve, 1)); };
function fixture(source = '[helm_console]\nturn_rate = 2\n', dependencies = { base_files: {}, packs: [] }) {
  document.body.innerHTML = '<main></main>';
  const draft = new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: '[pack]\nformat=1\nid="test"\nname="Test"\nversion="1"\n' }, { path, text: source }]));
  const runtime = { dependencies: async () => dependencies, validate: vi.fn(async () => ({ accepted: true })) }, preview = vi.fn(() => true), test = vi.fn(() => true);
  let busy = false; const panel = mountWorkshopHelmProjection({ root: document.querySelector('main'), runtime, draft: () => draft,
    busy: () => busy, setBusy: value => { busy = value; }, preview, test });
  return { draft, panel, runtime, preview, test, source };
}
const enable = () => { control('enabled').checked = true; control('enabled').dispatchEvent(new Event('change')); };
describe('Workshop Helm projection form', () => {
  it('adds optional estimates through acceptance and undo/reload, then explicitly disables', async () => {
    const { draft, panel, runtime, source } = fixture(); await settle();
    for (const input of panel.node.querySelectorAll('input,select')) expect(panel.node.querySelector(`label[for="${input.id}"]`)).not.toBeNull();
    expect(control('enabled').checked).toBe(false); expect(control('horizon').disabled).toBe(true);
    enable(); control('horizon').value = '45'; control('save').click(); await settle();
    expect(parse(draft.read(path)).helm_console.projection).toEqual({ horizon_secs: 45 }); expect(runtime.validate).toHaveBeenCalledOnce();
    draft.undo(); panel.refresh(); expect(draft.read(path)).toBe(source); expect(control('enabled').checked).toBe(false);
    draft.redo(); panel.refresh(); expect(control('horizon').value).toBe('45');
    control('enabled').checked = false; control('save').click(); await settle();
    expect(parse(draft.read(path)).helm_console.projection).toBe(false); panel.dispose();
  });
  it('preserves focused unsaved input on ordinary refresh and invokes existing Preview/Test', async () => {
    const { panel, preview, test } = fixture(); await settle(); enable(); const input = control('horizon');
    input.value = '17'; input.focus(); panel.refresh(); expect(control('horizon')).toBe(input); expect(document.activeElement).toBe(input); expect(input.value).toBe('17');
    control('preview').click(); control('test').click(); expect(preview).toHaveBeenCalledWith(path); expect(test).toHaveBeenCalledWith(path); panel.dispose();
  });
  it('requires inherited override consent and leaves refused source/history intact', async () => {
    const source = 'includes=["base.toml"]\n# child\n', { draft, panel, runtime } = fixture(source, { base_files: { 'assets/entities/base.toml': '[helm_console]\nprojection={horizon_secs=30}\nturn_rate=2\n' }, packs: [] });
    await settle(); expect(control('enabled').checked).toBe(true); control('enabled').checked = false; control('save').click(); await settle();
    expect(draft.read(path)).toBe(source); expect(runtime.validate).not.toHaveBeenCalled();
    control('materialise').checked = true; runtime.validate.mockResolvedValueOnce({ accepted: false, findings: [{ message: 'policy refused' }] }); control('save').click(); await settle();
    expect(control('status').textContent).toContain('policy refused'); expect(draft.snapshot().history.undo).toHaveLength(0);
    control('save').click(); await settle(); expect(parse(draft.read(path)).helm_console.projection).toBe(false); panel.dispose();
  });
  it('disables controls while accepting and drops an answer after disposal', async () => {
    const { draft, panel, runtime, source } = fixture(); await settle(); enable(); control('horizon').value = '10';
    let answer; runtime.validate.mockImplementationOnce(() => new Promise(resolve => { answer = resolve; })); control('save').click(); await settle();
    expect(control('save').disabled).toBe(true); expect(control('horizon').disabled).toBe(true); panel.dispose(); answer({ accepted: true }); await settle();
    expect(draft.read(path)).toBe(source); expect(draft.snapshot().history.undo).toHaveLength(0);
  });
  it('refuses malformed horizon locally and keeps a non-Helm template unavailable', async () => {
    const { panel, runtime } = fixture(); await settle(); enable(); control('horizon').value = '0'; control('save').click(); await settle();
    expect(runtime.validate).not.toHaveBeenCalled(); panel.dispose();
    const other = fixture('name="no helm"\n'); await settle(); expect(control('enabled').disabled).toBe(true); expect(control('save').disabled).toBe(true); other.panel.dispose();
  });
});
