/** @vitest-environment jsdom */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { WorkshopDocument } from '../../editor/workshop-document.js';
import { mountWorkshopSlotAuthoring } from '../../gui/workshop-slot-authoring-panel.js';

const world = 'assets/worlds/mission.toml', lead = 'assets/entities/lead.toml', wing = 'assets/entities/wing.toml';
const draft = () => new WorkshopDocument(createStoreZip([
  { path: 'scenarios.toml', text: '[pack]\nformat=1\nid="mine"\nversion="1"\nname="Mine"\n[pack.requires]\ncontent_id="phoenix-base"\ncontent_epoch=1\n' },
  { path: world, text: `[global]\ntitle='Fleet'\n[[ship_slot]]\nid='lead'\ndefault_ship='${lead}'\n[[ship_slot.ships]]\ntemplate_path='${lead}'\n` },
  { path: lead, text: 'tags=["ship"]\n' }, { path: wing, text: 'tags=["ship"]\n' },
]));
async function eventually(assertion) {
  let error;
  for (let i = 0; i < 30; i += 1) { try { assertion(); return; } catch (caught) { error = caught; await new Promise(resolve => setTimeout(resolve, 5)); } }
  throw error;
}

describe('Workshop slot editor controls', () => {
  let mounted;
  afterEach(() => mounted?.dispose());
  it('edits an authored slot with labelled keyboard controls and runtime checked feedback', async () => {
    document.body.innerHTML = '<main id="root"></main>';
    const current = draft(), changed = vi.fn(), runtime = { validate: vi.fn(async () => ({ accepted: true, findings: [] })) };
    mounted = mountWorkshopSlotAuthoring({ root: document.getElementById('root'), draft: () => current,
      runtime, busy: () => false, setBusy: vi.fn(), changed });
    for (const input of document.querySelectorAll('#workshop-slot-authoring input,#workshop-slot-authoring select')) {
      expect(document.querySelector(`label[for="${input.id}"]`)).not.toBeNull();
    }
    const slot = document.getElementById('workshop-slot-list'); slot.value = 'lead'; slot.dispatchEvent(new Event('change'));
    const allowed = document.getElementById('workshop-slot-ships');
    allowed.options[1].selected = true; allowed.dispatchEvent(new Event('change'));
    const fallback = document.getElementById('workshop-slot-default'); fallback.value = wing;
    document.getElementById('workshop-slot-unclaimed').value = 'absent';
    document.getElementById('workshop-slot-update').click();
    await eventually(() => expect(changed).toHaveBeenCalledWith(world));
    expect(current.read(world)).toContain(`default_ship = "${wing}"`);
    expect(current.read(world)).toContain('unclaimed = "absent"');
    expect(runtime.validate).toHaveBeenCalledOnce();
    expect(document.getElementById('workshop-slot-status').getAttribute('role')).toBe('status');
  });
});
