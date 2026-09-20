// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { WorkshopDocument } from '../../editor/workshop-document.js';
import { mountWorkshopSpatial } from '../../gui/workshop-spatial-panel.js';

const manifest = '[pack]\nformat=1\nid="mine"\nversion="1"\nname="Mine"\n[pack.requires]\ncontent_id="phoenix-base"\ncontent_epoch=1\n';
const source = '[anchors]\nstart=[1,0,2] # exact\n[[entity]]\ntemplate_path="assets/entities/region_nebula.toml"\nid="fog"\ntransform={position=[4,0,5],rotation=[0,1,0]}\n';
const makeDraft = () => new WorkshopDocument(createStoreZip([
  { path: 'scenarios.toml', text: manifest }, { path: 'assets/worlds/root.toml', text: source },
]));

describe('Workshop spatial panel', () => {
  let draft, runtime, changed, held, mounted;
  beforeEach(() => {
    document.body.innerHTML = '<main id="root"></main>'; draft = makeDraft(); changed = vi.fn(); held = false;
    runtime = { dependencies: vi.fn(async () => ({ base_files: {
      'assets/entities/region_nebula.toml': 'tags=["region"]\n[shape]\ntype="sphere"\nradius=5\n',
    }, packs: [] })), validate: vi.fn(async () => ({ accepted: true, findings: [] })) };
    mounted = mountWorkshopSpatial({ root: document.getElementById('root'), runtime, draft: () => draft,
      busy: () => held, setBusy: value => { held = value; }, changed });
  });

  it('offers a non-colour list/form alternative and focusable canvas markers', async () => {
    await vi.waitFor(() => expect(runtime.dependencies).toHaveBeenCalledOnce());
    mounted.refresh();
    expect(document.getElementById('workshop-spatial-anchors').options[0].textContent).toContain('start');
    expect(document.getElementById('workshop-spatial-entities').options[0].textContent).toContain('Region');
    const markers = [...document.querySelectorAll('.workshop-spatial-marker')];
    expect(markers).toHaveLength(2);
    expect(markers.map(row => row.dataset.kind)).toEqual(['anchor', 'region']);
    expect(markers.every(row => row.tagName === 'BUTTON')).toBe(true);
  });

  it('moves a selected Region and heading through exact candidate validation', async () => {
    const entities = document.getElementById('workshop-spatial-entities');
    entities.value = '0'; entities.dispatchEvent(new Event('change'));
    document.getElementById('workshop-spatial-x').value = '7';
    document.getElementById('workshop-spatial-z').value = '8';
    document.getElementById('workshop-spatial-yaw').value = '2';
    document.getElementById('workshop-spatial-entity-move').click();
    await vi.waitFor(() => expect(changed).toHaveBeenCalledWith('assets/worlds/root.toml'));
    expect(runtime.validate).toHaveBeenCalledOnce();
    expect(draft.read('assets/worlds/root.toml')).toContain('transform={position=[7, 0, 8],rotation=[0, 2, 0]}');
  });

  it('gives each canvas marker arrow-key placement and refuses invalid candidates without mutation', async () => {
    runtime.validate.mockResolvedValue({ accepted: false,
      findings: [{ file: 'assets/worlds/root.toml', message: 'outside bounds' }] });
    const marker = document.querySelector('[data-kind="anchor"]'); marker.focus();
    marker.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true }));
    const status = document.getElementById('workshop-spatial-status');
    await vi.waitFor(() => expect(status.getAttribute('role')).toBe('alert'));
    expect(status.textContent).toContain('outside bounds');
    expect(document.activeElement).toBe(status);
    expect(draft.read('assets/worlds/root.toml')).toBe(source);
    expect(changed).not.toHaveBeenCalled();
  });

  it('restores marker focus after a successful keyboard move rebuilds the canvas', async () => {
    const marker = document.querySelector('[data-kind="anchor"]'); marker.focus();
    marker.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true }));
    await vi.waitFor(() => expect(document.activeElement?.dataset.key).toBe('anchor:start'));
  });
});
