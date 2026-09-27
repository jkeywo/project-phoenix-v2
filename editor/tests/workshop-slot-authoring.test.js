import { describe, expect, it, vi } from 'vitest';
import { parse } from 'smol-toml';
import { createStoreZip } from '../mod-pack-export.js';
import { WorkshopDocument } from '../workshop-document.js';
import { applySlotOperation, inspectSlots, prepareSlotOperation } from '../workshop-slot-authoring.js';

const worldPath = 'assets/worlds/mission.toml';
const lead = 'assets/entities/lead.toml', wing = 'assets/entities/wing.toml';
const source = `[global]\ntitle = 'Two ships'\n# keep this note\n[[ship_slot]]\nid = 'lead'\ndefault_ship = '${lead}'\n[[ship_slot.ships]]\ntemplate_path = '${lead}'\n`;
const draft = () => new WorkshopDocument(createStoreZip([
  { path: 'scenarios.toml', text: '[pack]\nformat=1\nid="mine"\nversion="1"\nname="Mine"\n[pack.requires]\ncontent_id="phoenix-base"\ncontent_epoch=1\n' },
  { path: worldPath, text: source }, { path: lead, text: 'tags=["ship"]\n' }, { path: wing, text: 'tags=["ship"]\n' },
]));

describe('Workshop mission ship slot authoring', () => {
  it('validates before one undoable edit, then exports and reopens the slot choices', async () => {
    const current = draft(), runtime = { validate: vi.fn(async () => ({ accepted: true, findings: [] })) };
    const row = { id: 'wing', label: 'Wing', ships: [wing, lead], default_ship: wing, unclaimed: 'absent' };
    await applySlotOperation({ draft: current, runtime, path: worldPath, operation: { type: 'add', row } });
    expect(runtime.validate).toHaveBeenCalledOnce();
    expect(inspectSlots(current, worldPath)[1]).toEqual(row);
    expect(current.read(worldPath)).toContain('# keep this note');
    expect(current.canUndo()).toBe(true);
    current.undo();
    expect(current.read(worldPath)).toBe(source);
    current.redo();
    const reopened = new WorkshopDocument(current.archive());
    expect(inspectSlots(reopened, worldPath)[1]).toEqual(row);
    expect(parse(reopened.read(worldPath)).ship_slot).toHaveLength(2);
  });

  it('preserves a legacy world and refuses an invalid default before validation', () => {
    const current = draft();
    expect(() => prepareSlotOperation(current, worldPath, { type: 'add', row: { id: 'wing',
      ships: [wing], default_ship: lead, unclaimed: 'backfill' } })).toThrow('workshop.slot.error_default');
    expect(current.read(worldPath)).toBe(source);
  });

  it('updates only the selected slot and leaves the following world tables in place', () => {
    const current = draft();
    const suffix = `\n[[ship_slot]]\nid='wing'\ndefault_ship='${wing}'\n[[ship_slot.ships]]\ntemplate_path='${wing}'\n\n[[entity]]\nid='beacon'\ntemplate_path='assets/entities/beacon.toml'\n`;
    current.edit(worldPath, source + suffix);
    const change = prepareSlotOperation(current, worldPath, { type: 'update', target: 'lead',
      row: { id: 'lead', label: 'Lead', ships: [lead, wing], default_ship: lead, unclaimed: 'backfill' } });
    expect(change.after).toContain(suffix);
    expect(parse(change.after).ship_slot).toHaveLength(2);
    expect(parse(change.after).entity[0].id).toBe('beacon');
  });

  it('retains per-hull labels and ignores table-looking lines inside multiline text', () => {
    const current = draft();
    const labelled = `[global]\ntitle='Fleet'\n[[ship_slot]]\nid='lead'\nlabel="""Lead\n[[ship_slot]]\ntext"""\ndefault_ship='${lead}'\n[[ship_slot.ships]]\ntemplate_path='${lead}'\nlabel='world.ship.lead'\n\n[[entity]]\nid='beacon'\ntemplate_path='assets/entities/beacon.toml'\n`;
    current.edit(worldPath, labelled);
    const change = prepareSlotOperation(current, worldPath, { type: 'update', target: 'lead',
      row: { id: 'lead', label: 'Lead', ships: [lead, wing], default_ship: lead, unclaimed: 'backfill' } });
    const parsed = parse(change.after);
    expect(parsed.ship_slot).toHaveLength(1);
    expect(parsed.ship_slot[0].ships[0].label).toBe('world.ship.lead');
    expect(parsed.entity[0].id).toBe('beacon');
  });
});
