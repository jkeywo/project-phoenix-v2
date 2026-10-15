/** @vitest-environment jsdom */
import { expect, it, vi } from 'vitest';
import { parse } from 'smol-toml';
import { WorkshopDocument } from '../../editor/workshop-document.js';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { mountWorkshopHelmMovement } from '../../gui/workshop-helm-movement-panel.js';
const path = 'assets/entities/test.toml', control = id => document.getElementById('workshop-helm-movement-' + id);
const source = '# authored controls\n[[console]]\nid="helm"\n[[system]]\nid="drive"\nkind="helm_thrust"\nconsole="helm"\n[[system]]\nid="rudder"\nkind="helm_steering"\nconsole="helm"\n';
const settle = async () => { for (let i = 0; i < 10; i++) await new Promise(resolve => setTimeout(resolve, 1)); };
function fixture(text = source, dependencies = { base_files: {}, packs: [] }) {
  document.body.innerHTML = '<main></main>';
  const draft = new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: '[pack]\nformat=1\nid="test"\nname="Test"\nversion="1"\n' }, { path, text }]));
  const runtime = { dependencies: async () => dependencies, validate: vi.fn(async () => ({ accepted: true })) }, preview = vi.fn(() => true), test = vi.fn(() => true);
  let busy = false; const panel = mountWorkshopHelmMovement({ root: document.querySelector('main'), runtime, draft: () => draft,
    busy: () => busy, setBusy: value => { busy = value; }, preview, test });
  return { panel, draft, runtime, preview, test };
}
it('authors actual axis bindings, undo/reload and explicit disable through accepted history', async () => {
  const { panel, draft, runtime, preview, test } = fixture(); await settle();
  for (const input of panel.node.querySelectorAll('input,select')) expect(panel.node.querySelector(`label[for="${input.id}"]`)).not.toBeNull();
  control('enabled').checked = true; control('enabled').dispatchEvent(new Event('change'));
  const select = control('thrust'); select.focus(); panel.refresh(); expect(control('thrust')).toBe(select); expect(document.activeElement).toBe(select);
  control('save').click(); await settle(); expect(parse(draft.read(path)).helm_control).toEqual({ thrust: 'drive', steering: 'rudder' }); expect(runtime.validate).toHaveBeenCalledOnce();
  draft.undo(); panel.refresh(); expect(control('enabled').checked).toBe(false); draft.redo(); panel.refresh();
  control('preview').click(); control('test').click(); expect(preview).toHaveBeenCalledWith(path); expect(test).toHaveBeenCalledWith(path);
  control('enabled').checked = false; control('save').click(); await settle(); expect(parse(draft.read(path)).helm_control).toBe(false); panel.dispose();
});
it('requires materialisation consent and preserves source/history on refused acceptance', async () => {
  const child = 'includes=["base.toml"]\n# child\n', { panel, draft, runtime } = fixture(child, { base_files: { 'assets/entities/base.toml': source + '[helm_control]\nthrust="drive"\nsteering="rudder"\n' }, packs: [] });
  await settle(); control('enabled').checked = false; control('save').click(); await settle(); expect(draft.read(path)).toBe(child); expect(runtime.validate).not.toHaveBeenCalled();
  control('materialise').checked = true; runtime.validate.mockResolvedValueOnce({ accepted: false, findings: [{ message: 'owner refused' }] }); control('save').click(); await settle();
  expect(control('status').textContent).toContain('owner refused'); expect(draft.snapshot().history.undo).toHaveLength(0);
  control('save').click(); await settle(); expect(parse(draft.read(path)).helm_control).toBe(false); panel.dispose();
});
