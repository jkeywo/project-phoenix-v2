/** @vitest-environment jsdom */
import { describe, it, expect, vi } from 'vitest';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { WorkshopDocument } from '../../editor/workshop-document.js';
import { mountWorkshopResources } from '../../gui/workshop-resource-panel.js';
const path = 'assets/entities/depot.toml';
const source = '[torpedoes]\ncount=3\n[umbilical]\ncapacity="torpedoes"\nrate=1\ndirection="collect"\nmin_power_level=1\n[[infrastructure.capacity]]\nid="ammo"\namount=10\nceiling=20\n';
async function settle() { for (let i = 0; i < 20; i++) await new Promise(resolve => setTimeout(resolve, 1)); }
describe('Workshop resource form', () => {
  it('edits a real source through labelled native controls and shared history', async () => {
    document.body.innerHTML = '<main></main>';
    const current = new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: '[pack]\nformat=1\nid="mine"\nname="Mine"\nversion="1"\n' }, { path, text: source }]));
    const runtime = { dependencies: async () => ({ base_files: {}, packs: [] }), validate: vi.fn(async () => ({ accepted: true, findings: [] })) };
    const changed = vi.fn(), panel = mountWorkshopResources({ root: document.querySelector('main'), runtime, draft: () => current,
      busy: () => false, setBusy: vi.fn(), changed });
    await settle();
    for (const control of panel.node.querySelectorAll('input,select')) expect(panel.node.querySelector(`label[for="${control.id}"]`)).not.toBeNull();
    document.getElementById('workshop-resource-amount').value = '6';
    document.getElementById('workshop-resource-capacity-apply').click(); await settle();
    expect(document.getElementById('workshop-resource-status').textContent).not.toMatch(/refused/i);
    expect(current.read(path)).toContain('amount=6'); expect(changed).toHaveBeenCalledWith(path); expect(runtime.validate).toHaveBeenCalledTimes(1);
    current.undo(); panel.refresh(); expect(current.read(path)).toBe(source);
    document.getElementById('workshop-resource-service-name').value = 'torpedoes';
    document.getElementById('workshop-resource-service-kind').value = 'torpedo_magazine';
    document.getElementById('workshop-resource-service-apply').click(); await settle();
    expect(current.read(path)).toContain('kind = "torpedo_magazine"');
    panel.dispose();
  });
});
