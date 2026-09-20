import { describe, expect, it, vi } from 'vitest';
import { parse } from 'smol-toml';
import { createStoreZip } from '../mod-pack-export.js';
import { WorkshopDocument } from '../workshop-document.js';
import { applyShipOperation, inspectShipAuthoring, prepareShipOperation } from '../workshop-ship-authoring.js';

const manifest = '[pack]\nformat=1\nid="mine"\nversion="1"\nname="Mine"\n[pack.requires]\ncontent_id="phoenix-base"\ncontent_epoch=1\n';
const source = `# ship comments stay
tags = ["ship"]
extension = "keep"

[[station]] # bridge comment
id = "helm"
name = "Helm"
description = "Fly"
rank = "Lt."
console = "gui/helm.html"
unknown_station = 7

[[station.rating]]
name = "Assisted"
automated_systems = ["drive"] # rating comment

[[station]]
id = "science"
name = "Science"
description = "Scan"
rank = "Lt."

[[system]] # drive comment
id = "drive"
kind = "helm_thrust"
station = "helm"
unknown_system = true

[[system]]
id = "sensors"
kind = "sensors"
station = "science"
`;
const dependencies = { base_files: {}, packs: [] };
const schema = { system_kinds: ['helm_thrust', 'sensors'], directive_kinds: ['None', 'Destroy', 'Reach', 'Patrol'] };
const pack = () => new WorkshopDocument(createStoreZip([
  { path: 'scenarios.toml', text: manifest }, { path: 'assets/entities/ship.toml', text: source },
]));
const op = value => ({ path: 'assets/entities/ship.toml', ...value });

describe('Workshop playable ship exact-source transactions', () => {
  it('shows effective provenance and preserves comments, unknown fields and ordering on grouped edits', () => {
    const draft = pack();
    const inspection = inspectShipAuthoring(draft, dependencies, op({}).path);
    expect(inspection.stations.map(row => row.id)).toEqual(['helm', 'science']);
    expect(inspection.stations.every(row => row.local)).toBe(true);
    const prepared = prepareShipOperation(draft, dependencies, schema, op({ type: 'station-update', id: 'helm',
      fields: { name: 'Flight', console: 'gui/flight.html' } }));
    expect(prepared.changes).toHaveLength(1);
    expect(prepared.changes[0].after).toContain('[[station]] # bridge comment\nid = "helm"\nname = "Flight"');
    expect(prepared.changes[0].after).toContain('unknown_station = 7');
    expect(prepared.changes[0].after).toContain('automated_systems = ["drive"] # rating comment');
    expect(prepared.changes[0].after.indexOf('id = "helm"')).toBeLessThan(prepared.changes[0].after.indexOf('id = "science"'));
  });

  it('distinguishes included ship rows and allows local Systems to name included Stations', () => {
    const draft = pack();
    draft.edit(operationPath(), 'tags = ["ship"]\nincludes = ["base.toml"]\n');
    const inherited = { base_files: { 'assets/entities/base.toml': source }, packs: [] };
    const inspection = inspectShipAuthoring(draft, inherited, operationPath());
    expect(inspection.stations[0]).toMatchObject({ id: 'helm', local: false, owner: 'assets/entities/base.toml' });
    const prepared = prepareShipOperation(draft, inherited, schema, op({ type: 'system-add', id: 'second-drive', kind: 'helm_thrust', station: 'helm' }));
    expect(parse(prepared.changes[0].after).system[0]).toMatchObject({ id: 'second-drive', station: 'helm' });
  });

  it('adds, reorders and removes Stations and Systems without normalising untouched source', () => {
    const draft = pack();
    const added = prepareShipOperation(draft, dependencies, schema, op({ type: 'station-add', id: 'captain', name: 'Captain', description: 'Lead', rank: 'Cpt.' }));
    expect(parse(added.changes[0].after).station.map(row => row.id)).toEqual(['helm', 'science', 'captain']);
    draft.apply(added.changes);
    const moved = prepareShipOperation(draft, dependencies, schema, op({ type: 'station-move', id: 'captain', direction: -1 }));
    expect(parse(moved.changes[0].after).station.map(row => row.id)).toEqual(['helm', 'captain', 'science']);
    expect(moved.changes[0].after).toContain('# ship comments stay');
    const system = prepareShipOperation(draft, dependencies, schema, op({ type: 'system-add', id: 'command', kind: 'sensors', station: 'captain' }));
    expect(parse(system.changes[0].after).system.at(-1)).toMatchObject({ id: 'command', kind: 'sensors', station: 'captain' });
  });

  it('authors rating automation only against Systems owned by that Station', () => {
    const draft = pack();
    const accepted = prepareShipOperation(draft, dependencies, schema, op({ type: 'rating-set-systems', station: 'helm', rating: 'Assisted', systems: [] }));
    expect(accepted.changes[0].after).toContain('automated_systems = [] # rating comment');
    expect(() => prepareShipOperation(draft, dependencies, schema, op({ type: 'rating-set-systems', station: 'helm', rating: 'Assisted', systems: ['sensors'] })))
      .toThrow(/may automate only Systems owned by Station helm/);
    const second = prepareShipOperation(draft, dependencies, schema, op({ type: 'rating-add', station: 'helm', name: 'Manual' }));
    draft.apply(second.changes);
    const moved = prepareShipOperation(draft, dependencies, schema, op({ type: 'rating-move', station: 'helm', rating: 'Manual', direction: -1 }));
    expect(parse(moved.changes[0].after).station[0].rating.map(row => row.name)).toEqual(['Manual', 'Assisted']);
    draft.apply(moved.changes);
    const renamed = prepareShipOperation(draft, dependencies, schema, op({ type: 'rating-update', station: 'helm', rating: 'Manual', name: 'Direct', systems: ['drive'] }));
    expect(parse(renamed.changes[0].after).station[0].rating[0]).toMatchObject({ name: 'Direct', automated_systems: ['drive'] });
  });

  it('source-links unknown kinds and invalid Station membership before history changes', () => {
    const draft = pack();
    for (const operation of [
      op({ type: 'system-add', id: 'mystery', kind: 'made_up', station: 'helm' }),
      op({ type: 'system-add', id: 'orphan', kind: 'sensors', station: 'missing' }),
    ]) {
      try { prepareShipOperation(draft, dependencies, schema, operation); throw new Error('accepted invalid operation'); }
      catch (error) { expect(error.report.findings[0]).toMatchObject({ file: operation.path, line: expect.any(Number) }); }
    }
    expect(draft.read(operationPath())).toBe(source);
  });

  it('adds, edits, reorders and removes runtime-backed doctrine definitions', () => {
    const draft = pack();
    const first = prepareShipOperation(draft, dependencies, schema, op({ type: 'doctrine-add', id: 'hold', kind: 'None', base_priority: 2, references: {} }));
    draft.apply(first.changes);
    const second = prepareShipOperation(draft, dependencies, schema, op({ type: 'doctrine-add', id: 'fight', kind: 'Destroy', base_priority: 5,
      references: { directive_target: 'enemy' } }));
    draft.apply(second.changes);
    expect(parse(draft.read(operationPath())).behaviour.doctrine.map(row => row.id)).toEqual(['hold', 'fight']);
    const moved = prepareShipOperation(draft, dependencies, schema, op({ type: 'doctrine-move', id: 'fight', direction: -1 }));
    draft.apply(moved.changes);
    const updated = prepareShipOperation(draft, dependencies, schema, op({ type: 'doctrine-update', id: 'fight', kind: 'Patrol', base_priority: 8,
      references: { directive_anchors: ['alpha', 'beta'], directive_loop: true } }));
    expect(parse(updated.changes[0].after).behaviour.doctrine[0]).toMatchObject({ id: 'fight', directive_kind: 'Patrol',
      directive_anchors: ['alpha', 'beta'], directive_loop: true, base_priority: 8 });
    expect(updated.changes[0].after).not.toContain('directive_target = "enemy"');
    draft.apply(updated.changes);
    const removed = prepareShipOperation(draft, dependencies, schema, op({ type: 'doctrine-remove', id: 'hold' }));
    expect(parse(removed.changes[0].after).behaviour.doctrine.map(row => row.id)).toEqual(['fight']);
  });

  it('source-links inherited topology refusals to the owning file and line', () => {
    const draft = pack(); draft.edit(operationPath(), 'tags=["ship"]\nincludes=["base.toml"]\n');
    const inherited = { base_files: { 'assets/entities/base.toml': `${source}[behaviour]\n[[behaviour.doctrine]]\nid="hold"\nbase_priority=1\n` }, packs: [] };
    for (const operation of [op({ type: 'station-remove', id: 'helm' }), op({ type: 'station-move', id: 'helm', direction: 1 }),
      op({ type: 'system-remove', id: 'drive' }), op({ type: 'system-move', id: 'drive', direction: 1 }),
      op({ type: 'rating-remove', station: 'helm', rating: 'Assisted' }), op({ type: 'rating-move', station: 'helm', rating: 'Assisted', direction: 1 }),
      op({ type: 'doctrine-remove', id: 'hold' }), op({ type: 'doctrine-move', id: 'hold', direction: 1 })]) {
      try { prepareShipOperation(draft, inherited, schema, operation); throw new Error('accepted inherited edit'); }
      catch (error) { expect(error.report.findings[0]).toMatchObject({ file: 'assets/entities/base.toml', line: expect.any(Number) }); }
    }
  });

  it('commits one grouped undo entry only after ordinary runtime acceptance and rejects stale completions', async () => {
    const draft = pack(), runtime = { dependencies: vi.fn(async () => dependencies), shipSchema: vi.fn(async () => schema),
      validate: vi.fn(async () => ({ accepted: true, findings: [] })) };
    await applyShipOperation({ draft, runtime, operation: op({ type: 'system-update', id: 'drive', fields: { kind: 'helm_thrust', station: 'helm', ai_only: false } }) });
    expect(draft.snapshot().history.undo).toHaveLength(1);
    draft.undo(); expect(draft.read(operationPath())).toBe(source);
    await expect(applyShipOperation({ draft, runtime, operation: op({ type: 'system-update', id: 'drive', fields: { kind: 'helm_thrust' } }), current: () => false }))
      .rejects.toThrow('stale-ship-operation');
  });
});

function operationPath() { return 'assets/entities/ship.toml'; }
