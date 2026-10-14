/** @vitest-environment jsdom */
import { describe, it, expect, vi } from 'vitest';
import { parse } from 'smol-toml';
import { WorkshopDocument } from '../../editor/workshop-document.js';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { interiorSource } from '../../editor/tests/interior-fixture.js';
import { mountWorkshopInterior } from '../../gui/workshop-interior-panel.js';
const path = 'assets/entities/ship.toml', control = id => document.getElementById('workshop-interior-' + id);
async function settle() { for (let i = 0; i < 15; i++) await new Promise(resolve => setTimeout(resolve, 1)); }
const change = (id, value, type = 'change') => { control(id).value = value; control(id).dispatchEvent(new Event(type, { bubbles: true })); };
function fixture(source = interiorSource()) {
  document.body.innerHTML = '<main></main>';
  const draft = new WorkshopDocument(createStoreZip([
    { path: 'scenarios.toml', text: '[pack]\nformat=1\nid="mine"\nname="Mine"\nversion="1"\n' }, { path, text: source }]));
  const runtime = { dependencies: async () => ({ base_files: {}, packs: [] }), validate: vi.fn(async () => ({ accepted: true, findings: [] })) };
  const changed = vi.fn(), panel = mountWorkshopInterior({ root: document.querySelector('main'), runtime, draft: () => draft,
    busy: () => false, setBusy: vi.fn(), changed });
  return { source, draft, runtime, changed, panel };
}
describe('Workshop physical layout form', () => {
  it('edits explicit structural recovery data without legacy repair buckets', async () => {
    const { draft, panel } = fixture(); await settle();
    expect(control('repair-target')).toBeNull();
    change('max_hp', '75', 'input'); change('recovery_fraction', '0.3', 'input');
    control('save').click(); await settle();
    expect(parse(draft.read(path)).interior.compartment[0]).toMatchObject({ max_hp: 75, recovery_fraction: 0.3, structural_remedy: 'structural' });
    panel.dispose();
  });
  it('labels every property and stages precise geometry, material, connections and placements as one edit', async () => {
    const { source, draft, runtime, changed, panel } = fixture(); await settle();
    for (const input of panel.node.querySelectorAll('input,select')) expect(panel.node.querySelector(`label[for="${input.id}"]`)).not.toBeNull();
    change('x', '-18', 'input'); change('material', 'unusual-alloy', 'input'); change('travel-secs', '7');
    change('system', 'helm'); change('placement', 'engineering');
    expect(draft.read(path)).toBe(source); control('save').click(); await settle();
    const profile = parse(draft.read(path)).interior;
    expect(profile.compartment[0]).toMatchObject({ x: -18, material: 'unusual-alloy' });
    expect(profile.connection[0].travel_secs).toBe(7); expect(profile.placements.helm).toBe('engineering');
    expect(runtime.validate).toHaveBeenCalledOnce(); expect(changed).toHaveBeenCalledWith(path);
    draft.undo(); panel.refresh(); expect(draft.read(path)).toBe(source); draft.redo(); panel.refresh(); expect(control('x').value).toBe('-18'); panel.dispose();
  });
  it('adds a deck and compartment, connects it, and persists an explicitly selected new team home together', async () => {
    const { draft, panel } = fixture(); await settle(); control('add-deck').click();
    change('deck-label', 'Cargo deck', 'input'); control('add-room').click(); change('label', 'Cargo room', 'input');
    control('add-connection').click(); change('to', 'room-1');
    control('add-team').click(); expect(control('home-1').value).toBe(''); change('home-1', 'room-1');
    control('save').click(); await settle(); const value = parse(draft.read(path));
    expect(value.repair.repair_team_count).toBe(2); expect(value.interior.team_homes).toEqual(['engineering', 'room-1']);
    expect(value.interior.compartment.at(-1)).toMatchObject({ deck: 'deck-1', label: 'Cargo room' });
    expect(value.interior.connection.at(-1).to).toBe('room-1'); expect(draft.snapshot().history.undo).toHaveLength(1); panel.dispose();
  });
  it('requires removing references before deleting rooms and refuses disconnected candidates', async () => {
    const { source, draft, panel } = fixture(); await settle(); control('remove-room').click();
    expect(control('status').textContent).toContain('Reassign'); control('remove-connection').click(); control('save').click(); await settle();
    expect(draft.read(path)).toBe(source); expect(control('status').textContent).toContain('Connect every compartment'); panel.dispose();
  });
  it('keeps source and history intact when the authoritative validator rejects a layout', async () => {
    const { source, draft, runtime, panel } = fixture(); await settle();
    runtime.validate.mockResolvedValueOnce({ accepted: false, findings: [{ message: 'No supported boundary component' }] });
    expect([...control('boundary').options].map(option => option.value)).toEqual(['']);
    change('x', '-12', 'input'); control('save').click(); await settle();
    expect(draft.read(path)).toBe(source); expect(draft.snapshot().history.undo).toHaveLength(0);
    expect(control('status').textContent).toContain('No supported boundary component'); panel.dispose();
  });
  it('uses actual SVG pointer movement and corner resizing to stage geometry without committing', async () => {
    const { source, draft, panel } = fixture(); await settle(); const svg = control('map');
    svg.getBoundingClientRect = () => ({ width: 320, height: 320, x: 0, y: 0 });
    const pointer = (target, type, x, y) => { const event = new Event(type, { bubbles: true }); Object.assign(event, { pointerId: 1, clientX: x, clientY: y }); target.dispatchEvent(event); };
    pointer(svg.querySelector('rect[role="button"]'), 'pointerdown', 20, 20); pointer(svg, 'pointermove', 52, 36); pointer(svg, 'pointerup', 52, 36);
    expect(Number(control('x').value)).toBeGreaterThan(-10); expect(Number(control('z').value)).toBeGreaterThan(0);
    pointer(svg.querySelector('[data-resize]'), 'pointerdown', 20, 20); pointer(svg, 'pointermove', 52, 52); pointer(svg, 'pointerup', 52, 52);
    expect(Number(control('width').value)).toBeGreaterThan(10); expect(Number(control('depth').value)).toBeGreaterThan(10);
    expect(draft.read(path)).toBe(source); control('save').click(); await settle(); expect(parse(draft.read(path)).interior.compartment[0].width).toBeGreaterThan(10); panel.dispose();
  });
  it('selects map rooms with keyboard and moves them between decks through labelled controls', async () => {
    const { draft, panel } = fixture(); await settle();
    control('map').querySelector('[role="button"]').dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    expect(document.activeElement.dataset.room).toBe('bridge');
    expect(control('map').querySelector('[data-resize]').getAttribute('fill')).toBe('white');
    change('room-deck', 'lower'); expect(control('decks').value).toBe('lower');
    control('save').click(); await settle(); expect(parse(draft.read(path)).interior.compartment[0].deck).toBe('lower'); panel.dispose();
  });
});
