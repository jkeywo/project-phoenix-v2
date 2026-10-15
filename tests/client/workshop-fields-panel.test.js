/** @vitest-environment jsdom */
import { describe, expect, it, vi } from 'vitest';
import { parse } from 'smol-toml';
import { WorkshopDocument } from '../../editor/workshop-document.js';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { mountWorkshopFields } from '../../gui/workshop-fields-panel.js';
const path = 'assets/entities/test.toml', source = 'tags=["region"]\n[shape]\ntype="sphere"\nradius=10\n';
const control = id => document.getElementById('workshop-fields-' + id);
const settle = async () => { for (let i=0;i<15;i++) await new Promise(resolve => setTimeout(resolve,1)); };
function fixture() {
  document.body.innerHTML = '<main></main>'; const draft = new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: '[pack]\nformat=1\nid="test"\nname="Test"\nversion="1"\n' }, { path, text: source }]));
  const runtime = { dependencies: async () => ({ base_files: {}, packs: [] }), validate: vi.fn(async () => ({ accepted: true })) };
  const panel = mountWorkshopFields({ root: document.querySelector('main'), runtime, draft: () => draft, busy: () => false, setBusy: vi.fn() }); return { draft, runtime, panel };
}
describe('Workshop field/terrain controls', () => {
  it('authors falloff, bounded evolution and signal-specific material through one accepted history operation', async () => {
    const { draft, runtime, panel } = fixture(); await settle();
    for (const input of panel.node.querySelectorAll('input,select')) expect(input.labels.length).toBeGreaterThan(0);
    control('field').checked = true; control('falloff').value = 'smooth'; control('sensor-absorption').value = '0.2'; control('evolution').checked = true;
    control('translation-z').value = '20'; control('material').checked = true; control('comms-opaque').checked = true; control('apply').click(); await settle();
    expect(runtime.validate).toHaveBeenCalledOnce(); expect(parse(draft.read(path)).field).toMatchObject({ falloff: 'smooth', evolution: { translation: [0,0,20] } });
    expect(parse(draft.read(path)).signal_material).toMatchObject({ comms_opaque: true, sensor_opaque: false });
    draft.undo(); expect(draft.read(path)).toBe(source); panel.dispose();
  });
  it('keeps exact source/history and edited form on authoritative refusal', async () => {
    const { draft, runtime, panel } = fixture(); await settle(); runtime.validate.mockResolvedValueOnce({ accepted: false, findings: [{ message: 'shape refused' }] });
    control('field').checked = true; control('strength').value = '0.75'; control('apply').click(); await settle(); panel.refresh();
    expect(draft.read(path)).toBe(source); expect(draft.snapshot().history.undo).toHaveLength(0); expect(control('strength').value).toBe('0.75');
    expect(panel.node.textContent).toContain('shape refused'); panel.dispose();
  });
});
