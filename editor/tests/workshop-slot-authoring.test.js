import { describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { createStoreZip } from '../mod-pack-export.js';
import { WorkshopDocument } from '../workshop-document.js';
import { applySlotOperation, inspectSlots, prepareSlotOperation, slotHullCatalogue } from '../workshop-slot-authoring.js';

const cases = JSON.parse(readFileSync(new URL('../../tests/fixtures/workshop-slot-edits.json', import.meta.url), 'utf8'));
const world = 'assets/worlds/mission.toml', base = 'assets/entities/base.toml', other = 'assets/entities/other.toml';
const manifest = '[pack]\nformat=1\nid="mine"\nversion="1"\nname="Mine"\n[pack.requires]\ncontent_id="phoenix-base"\ncontent_epoch=1\n';
function draft(source, native = false) {
  const files = { 'scenarios.toml': manifest, [world]: source, 'assets/strings/strings.csv': 'id,en\n',
    'assets/models/ship.glb': native ? { asset: '0123456789abcdef-3', length: 3 } : new Uint8Array([1, 2, 3]) };
  return native ? WorkshopDocument.fromNativeFiles(files, { kind: 'mod' })
    : new WorkshopDocument(createStoreZip(Object.entries(files).map(([path, value]) => typeof value === 'string' ? { path, text: value } : { path, bytes: value })));
}
const hulls = [{ path: base, origin: { kind: 'base' } }, { path: other, origin: { kind: 'pack', id: 'other' } }];
const dependencies = { base_files: { [base]: "class='lancer'\n" }, packs: [{ id: 'other', files: { [other]: "includes=['base.toml']\n" } }] };
const addedSource = cases[1].source + `\r\n[[ship_slot]]\r\nid="wing"\r\nlabel="Wing"\r\ndefault_ship="${base}"\r\nunclaimed="absent"\r\n[[ship_slot.ships]]\r\ntemplate_path="${base}"\r\n`;

describe('mission slot exact-source plans and runtime acceptance', () => {
  it.each(cases)('matches the real Rust editor fixture for $name', fixture => {
    const request = prepareSlotOperation(draft(fixture.source), world, fixture.operation, hulls);
    expect(request).toEqual({ document_path: world, expected_source: fixture.source, edits: fixture.edits });
  });

  it('keeps multiline labels and retained offers untouched while appending another hull', () => {
    const source = cases[0].source.replace("label='Original'", 'label="""Original\n[[ship_slot]]\ntext"""');
    const current = draft(source), row = { ...inspectSlots(current, world)[0], ships: [base, other] };
    expect(prepareSlotOperation(current, world, { type: 'update', target: 'lead', row }, hulls).edits).toEqual([
      { op: 'append_table', path: ['ship_slot', 0, 'ships'], fields: [['template_path', JSON.stringify(other)]] },
    ]);
  });

  it('removes only deselected offers and refuses unavailable hulls or an invalid default', () => {
    const current = draft(cases[0].source), row = { ...inspectSlots(current, world)[0], ships: [other], default_ship: other };
    expect(prepareSlotOperation(current, world, { type: 'update', target: 'lead', row }, hulls).edits).toEqual([
      { op: 'set', path: ['ship_slot', 0, 'default_ship'], value_source: JSON.stringify(other) },
      { op: 'remove', path: ['ship_slot', 0, 'ships', 0] },
      { op: 'append_table', path: ['ship_slot', 0, 'ships'], fields: [['template_path', JSON.stringify(other)]] },
    ]);
    expect(() => prepareSlotOperation(current, world, { type: 'update', target: 'lead', row: { ...row, default_ship: base } }, hulls)).toThrow('workshop.slot.error_default');
    expect(() => prepareSlotOperation(current, world, { type: 'update', target: 'lead', row }, hulls.slice(0, 1))).toThrow('workshop.slot.error_ships');
  });

  for (const native of [false, true]) {
    const provider = native ? { save() {}, restoreDocument: snapshot => WorkshopDocument.restore(snapshot, { native: true }),
      test: { catalog: async files => { expect(files['assets/strings/strings.csv']).toBeUndefined(); return { ships: [base, other] }; } } } : undefined;
    function setup() {
      const current = draft(cases[1].source, native);
      const runtime = { dependencies: async () => structuredClone(dependencies), testCatalog: async () => ({ ships: [base, other] }),
        edit: vi.fn(async (_source, request) => { expect(request.edits).toEqual(cases[1].edits); return addedSource; }),
        validate: vi.fn(async () => ({ accepted: true, findings: [] })) };
      return { draft: current, runtime, provider, path: world, operation: cases[1].operation };
    }
    it(`adopts a dependency hull once without copying dependency members (${native ? 'native' : 'browser'})`, async () => {
      const context = setup(), before = context.draft.members();
      await applySlotOperation(context);
      expect(context.draft.paths()).not.toContain(base);
      expect(context.draft.snapshot().history.undo).toHaveLength(1);
      expect(context.runtime.validate.mock.calls[0][0] === null).toBe(native);
      const reopened = native ? WorkshopDocument.restore(context.draft.snapshot(), { native: true }) : new WorkshopDocument(context.draft.archive());
      expect(inspectSlots(reopened, world).find(slot => slot.id === 'wing')).toEqual(cases[1].operation.row);
      context.draft.undo(); expect(context.draft.members()).toEqual(before);
    });
    it(`refuses stale validation and preserves other changes (${native ? 'native' : 'browser'})`, async () => {
      const context = setup(); let intervened;
      context.runtime.validate.mockImplementation(async () => {
        context.draft.put('assets/entities/later.toml', '# changed\n'); intervened = context.draft.snapshot();
        return { accepted: true, findings: [] };
      });
      await expect(applySlotOperation(context)).rejects.toThrow('workshop.slot.error_stale');
      expect(context.draft.snapshot()).toEqual(intervened);
    });
    it(`returns runtime findings without history on refusal (${native ? 'native' : 'browser'})`, async () => {
      const context = setup(), before = context.draft.snapshot();
      const report = { accepted: false, findings: [{ file: world, line: 1, message: 'refused' }] };
      context.runtime.validate.mockResolvedValue(report);
      await expect(applySlotOperation(context)).rejects.toMatchObject({ message: 'workshop.slot.error_runtime', report });
      expect(context.draft.snapshot()).toEqual(before);
    });
    it(`refuses a late source-editor result before validation (${native ? 'native' : 'browser'})`, async () => {
      const context = setup(); let intervened;
      context.runtime.edit.mockImplementation(async () => {
        context.draft.put('assets/entities/later.toml', '# changed\n'); intervened = context.draft.snapshot();
        return addedSource;
      });
      await expect(applySlotOperation(context)).rejects.toThrow('workshop.slot.error_stale');
      expect(context.runtime.validate).not.toHaveBeenCalled();
      expect(context.draft.snapshot()).toEqual(intervened);
    });
  }

  it('uses runtime eligibility over winning source origins and excludes non-source members', async () => {
    const current = draft(cases[1].source);
    current.put(base, 'includes=["fragment.toml"]\n');
    const runtime = { dependencies: async () => dependencies, testCatalog: vi.fn(async files => {
      expect(files[base]).toBe(current.read(base));
      expect(files[other]).toBe(dependencies.packs[0].files[other]);
      expect(files['assets/strings/strings.csv']).toBeUndefined();
      return { ships: [base, other] };
    }) };
    expect(await slotHullCatalogue({ draft: current, runtime })).toEqual([
      { path: base, origin: { kind: 'draft' } }, { path: other, origin: { kind: 'pack', id: 'other' } },
    ]);
  });
});
