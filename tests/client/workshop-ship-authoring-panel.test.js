/** @vitest-environment jsdom */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { WorkshopDocument } from '../../editor/workshop-document.js';
import { mountWorkshopShipAuthoring } from '../../gui/workshop-ship-authoring-panel.js';

const source = `tags=["ship"]
[[station]]
id="helm"
name="Helm"
description="Fly"
rank="Lt."
console="gui/helm.html"
[[station.rating]]
name="Assisted"
automated_systems=[]
[[system]]
id="drive"
kind="helm_thrust"
station="helm"
`;
const manifest = '[pack]\nformat=1\nid="mine"\nversion="1"\nname="Mine"\n[pack.requires]\ncontent_id="phoenix-base"\ncontent_epoch=1\n';
const draft = () => new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: manifest },
  { path: 'assets/entities/ship.toml', text: source }]));
async function eventually(assertion) {
  let error;
  for (let index = 0; index < 20; index += 1) {
    try { assertion(); return; } catch (caught) { error = caught; await new Promise(resolve => setTimeout(resolve, 5)); }
  }
  throw error;
}
describe('Workshop playable ship panel', () => {
  let mounted;
  afterEach(() => mounted?.dispose());
  it('uses labelled native controls, textual provenance and one runtime-validated keyboard action', async () => {
    document.body.innerHTML = '<main id="root"></main>'; const current = draft();
    const runtime = { dependencies: vi.fn(async () => ({ base_files: {}, packs: [] })),
      shipSchema: vi.fn(async () => ({ system_kinds: ['helm_thrust', 'sensors'], directive_kinds: ['None', 'Patrol'] })),
      validate: vi.fn(async () => ({ accepted: true, findings: [] })) };
    const changed = vi.fn();
    mounted = mountWorkshopShipAuthoring({ root: document.getElementById('root'), runtime, draft: () => current,
      busy: () => false, setBusy: vi.fn(), changed });
    await eventually(() => expect(document.getElementById('workshop-ship-stations').options.length).toBe(1));
    mounted.refresh();
    expect(document.getElementById('workshop-ship-station-apply').disabled,
      `${document.getElementById('workshop-ship-status').textContent} / ${document.getElementById('workshop-ship-stations').textContent}`).toBe(false);
    expect(document.querySelector('label[for="workshop-ship-system-kind"]')).not.toBeNull();
    expect(document.querySelector('label[for="workshop-ship-stations"]')).not.toBeNull();
    expect(document.querySelector('label[for="workshop-ship-systems"]')).not.toBeNull();
    expect(document.querySelector('label[for="workshop-ship-ratings"]')).not.toBeNull();
    expect([...document.getElementById('workshop-ship-system-kind').options].map(option => option.value)).toEqual(['helm_thrust', 'sensors']);
    expect([...document.getElementById('workshop-ship-doctrine-kind').options].map(option => option.value)).toEqual(['None', 'Patrol']);
    expect(document.getElementById('workshop-ship-stations').textContent).toContain('local');
    const name = document.getElementById('workshop-ship-station-name'); name.value = 'Flight';
    document.getElementById('workshop-ship-station-apply').click();
    await eventually(() => expect(changed).toHaveBeenCalledWith('assets/entities/ship.toml'));
    expect(current.read('assets/entities/ship.toml')).toContain('name="Flight"');
    expect(current.read('assets/entities/ship.toml')).toContain('console="gui/helm.html"');
    expect(runtime.validate).toHaveBeenCalledOnce();
    changed.mockClear();
    document.getElementById('workshop-ship-doctrine-id').value = 'hold';
    document.getElementById('workshop-ship-doctrine-priority').value = '3';
    document.getElementById('workshop-ship-doctrine-add').click();
    await eventually(() => expect(changed).toHaveBeenCalledWith('assets/entities/ship.toml'));
    expect(current.read('assets/entities/ship.toml')).toContain('[[behaviour.doctrine]]');
    expect(current.read('assets/entities/ship.toml')).toContain('id = "hold"');
    document.getElementById('workshop-ship-doctrine-kind').value = 'Patrol';
    document.getElementById('workshop-ship-doctrine-reference').value = 'alpha, beta';
    document.getElementById('workshop-ship-doctrine-loop').checked = true;
    changed.mockClear(); document.getElementById('workshop-ship-doctrine-apply').click();
    await eventually(() => expect(changed).toHaveBeenCalledWith('assets/entities/ship.toml'));
    expect(current.read('assets/entities/ship.toml')).toContain('directive_loop = true');
    expect(current.read('assets/entities/ship.toml')).toContain('directive_anchors = ["alpha", "beta"]');
    for (const button of document.querySelectorAll('#workshop-ship-authoring button')) expect(button.type).toBe('button');
  });
});
