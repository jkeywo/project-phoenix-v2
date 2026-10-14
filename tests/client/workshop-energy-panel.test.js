/** @vitest-environment jsdom */
import { describe, it, expect, vi } from 'vitest';
import { parse, stringify } from 'smol-toml';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { WorkshopDocument } from '../../editor/workshop-document.js';
import { createEnergyModel } from '../../editor/workshop-energy.js';
import { mountWorkshopEnergy } from '../../gui/workshop-energy-panel.js';
const path = 'assets/entities/energy-ship.toml';
const hardware = { system: [{ id: 'impulse', kind: 'impulse', power_group: 'propulsion' }, { id: 'power-reactor', kind: 'power_reactor', power_group: 'reactor' }], power_groups: { propulsion: { label: 'Propulsion' }, reactor: { label: 'Reactor' } },
  resource_services: { coolant: { kind: 'capacity', id: 'tank' } }, infrastructure: { capacity: [{ id: 'tank', amount: 10, ceiling: 20 }] } };
const source = stringify({ ...hardware, energy: createEnergyModel(hardware) });
async function settle() { for (let i = 0; i < 20; i++) await new Promise(resolve => setTimeout(resolve, 1)); }
const control = id => document.getElementById('workshop-energy-' + id);
function fixture(text = source) {
  document.body.innerHTML = '<main></main>';
  const current = new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: '[pack]\nformat=1\nid="mine"\nname="Mine"\nversion="1"\n' }, { path, text }]));
  const runtime = { dependencies: async () => ({ base_files: {}, packs: [] }), validate: vi.fn(async () => ({ accepted: true, findings: [] })) };
  const changed = vi.fn(), panel = mountWorkshopEnergy({ root: document.querySelector('main'), runtime, draft: () => current, busy: () => false, setBusy: vi.fn(), changed });
  return { current, runtime, changed, panel };
}
describe('Workshop energy form', () => {
  it('edits real consumer reserves through labelled controls and shared history', async () => {
    const { current, runtime, changed, panel } = fixture(); await settle();
    control('subject').value = 'consumer:impulse'; control('subject').dispatchEvent(new Event('change'));
    for (const input of panel.node.querySelectorAll('input,select')) expect(panel.node.querySelector(`label[for="${input.id}"]`)).not.toBeNull();
    control('reserve-capacity').value = '200'; control('field-discrete_cost').value = '4'; control('apply').click(); await settle();
    expect(parse(current.read(path)).energy.consumer[0]).toMatchObject({ reserve: { capacity: 200 }, discrete_cost: 4 });
    expect(changed).toHaveBeenCalledWith(path); expect(runtime.validate).toHaveBeenCalledTimes(1); current.undo(); panel.refresh(); expect(current.read(path)).toBe(source); panel.dispose();
  });
  it('edits coolant and nested control groups; authoritative refusal leaves draft unchanged', async () => {
    const { current, runtime, panel } = fixture(); await settle();
    control('subject').value = 'group:propulsion'; control('subject').dispatchEvent(new Event('change'));
    control('parent').value = 'reactor'; control('field-priority').value = '2'; control('field-restore_delay_secs').value = '1.5'; control('apply').click(); await settle();
    expect(parse(current.read(path)).energy.group.find(row => row.id === 'propulsion')).toMatchObject({ parent: 'reactor', priority: 2, restore_delay_secs: 1.5 });
    control('subject').value = 'reactor:'; control('subject').dispatchEvent(new Event('change'));
    control('endpoint-kind').value = 'capacity'; control('endpoint-kind').dispatchEvent(new Event('change')); control('endpoint').value = 'tank';
    control('field-max_cooling_per_sec').value = '6'; control('field-default_cooling_per_sec').value = '2';
    control('apply').click(); await settle(); expect(parse(current.read(path)).energy.reactor.coolant).toEqual({ provider: { kind: 'capacity', id: 'tank' } });
    expect(parse(current.read(path)).energy.reactor).toMatchObject({ max_cooling_per_sec: 6, default_cooling_per_sec: 2 });
    const before = current.read(path); runtime.validate.mockResolvedValueOnce({ accepted: false, findings: [{ message: 'initial energy exceeds capacity' }] });
    control('field-initial_energy').value = '999'; control('apply').click(); await settle(); expect(current.read(path)).toBe(before);
    expect(control('status').textContent).toContain('initial energy exceeds capacity'); panel.dispose();
  });
  it('creates a model only after explicit action and excludes reactor projection aliases', async () => {
    const before = stringify(hardware), { current, panel } = fixture(before); await settle(); expect(current.read(path)).toBe(before);
    control('create').click(); await settle(); expect(parse(current.read(path)).energy.consumer.map(item => item.system)).toEqual(['impulse']);
    expect(control('create').disabled).toBe(true); expect(control('apply').disabled).toBe(false); panel.dispose();
  });
});
