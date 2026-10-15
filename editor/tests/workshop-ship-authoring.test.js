import { describe, expect, it, vi } from 'vitest';
import fs from 'node:fs';
import {resolveTemplate} from '../entity-includes.js';
import { parse, stringify } from 'smol-toml';
import { createStoreZip } from '../mod-pack-export.js';
import { WorkshopDocument } from '../workshop-document.js';
import { applyShipOperation, inspectShipAuthoring, prepareShipOperation } from '../workshop-ship-authoring.js';
import { initialTurret, initialIllumination, initialJamming, initialProtection } from '../workshop-system-composition.js';

const manifest = '[pack]\nformat=1\nid="mine"\nversion="1"\nname="Mine"\n[pack.requires]\ncontent_id="phoenix-base"\ncontent_epoch=1\n';
const source = `# ship comments stay
tags = ["ship"]
extension = "keep"

[[station]] # bridge comment
id = "helm"
name = "Helm"
description = "Fly"
rank = "Lt."
unknown_station = 7

[[station.rating]]
name = "Assisted"
automated_systems = ["drive"] # rating comment

[[station]]
id = "science"
name = "Science"
description = "Scan"
rank = "Lt."

[[console]] # screen comment
id = "flight-screen"
home_station = "helm"
screen = "gui/helm.html"

[[console]]
id = "scan-screen"
home_station = "science"
screen = "gui/sensors.html"

[[system]] # drive comment
id = "drive"
kind = "helm_thrust"
console = "flight-screen"
unknown_system = true

[[system]]
id = "sensors"
kind = "sensors"
console = "scan-screen"
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
    expect(inspection.consoles.map(row => row.id)).toEqual(['flight-screen', 'scan-screen']);
    const prepared = prepareShipOperation(draft, dependencies, schema, op({ type: 'station-update', id: 'helm',
      fields: { name: 'Flight', description: 'Pilot the ship' } }));
    expect(prepared.changes).toHaveLength(1);
    expect(prepared.changes[0].after).toContain('[[station]] # bridge comment\nid = "helm"\nname = "Flight"');
    expect(prepared.changes[0].after).toContain('unknown_station = 7');
    expect(prepared.changes[0].after).toContain('automated_systems = ["drive"] # rating comment');
    expect(prepared.changes[0].after.indexOf('id = "helm"')).toBeLessThan(prepared.changes[0].after.indexOf('id = "science"'));
  });

  it('distinguishes included ship rows and allows local Systems to name included Consoles', () => {
    const draft = pack();
    draft.edit(operationPath(), 'tags = ["ship"]\nincludes = ["base.toml"]\n');
    const inherited = { base_files: { 'assets/entities/base.toml': source }, packs: [] };
    const inspection = inspectShipAuthoring(draft, inherited, operationPath());
    expect(inspection.stations[0]).toMatchObject({ id: 'helm', local: false, owner: 'assets/entities/base.toml' });
    expect(inspection.consoles[0]).toMatchObject({ id: 'flight-screen', local: false, owner: 'assets/entities/base.toml' });
    const prepared = prepareShipOperation(draft, inherited, schema, op({ type: 'system-add', id: 'second-drive', kind: 'helm_thrust', console: 'flight-screen' }));
    expect(parse(prepared.changes[0].after).system[0]).toMatchObject({ id: 'second-drive', console: 'flight-screen' });
  });

  it('adds, reorders and removes Stations and Systems without normalising untouched source', () => {
    const draft = pack();
    const added = prepareShipOperation(draft, dependencies, schema, op({ type: 'station-add', id: 'captain', name: 'Captain', description: 'Lead', rank: 'Cpt.' }));
    expect(parse(added.changes[0].after).station.map(row => row.id)).toEqual(['helm', 'science', 'captain']);
    draft.apply(added.changes);
    const moved = prepareShipOperation(draft, dependencies, schema, op({ type: 'station-move', id: 'captain', direction: -1 }));
    expect(parse(moved.changes[0].after).station.map(row => row.id)).toEqual(['helm', 'captain', 'science']);
    expect(moved.changes[0].after).toContain('# ship comments stay');
    const console = prepareShipOperation(draft, dependencies, schema, op({ type: 'console-add', id: 'command-screen', fields: { home_station: 'captain', screen: 'gui/sensors.html' } }));
    draft.apply(console.changes);
    const system = prepareShipOperation(draft, dependencies, schema, op({ type: 'system-add', id: 'command', kind: 'sensors', console: 'command-screen' }));
    expect(parse(system.changes[0].after).system.at(-1)).toMatchObject({ id: 'command', kind: 'sensors', console: 'command-screen' });
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

  it('source-links unknown kinds and invalid Console membership before history changes', () => {
    const draft = pack();
    for (const operation of [
      op({ type: 'system-add', id: 'mystery', kind: 'made_up', console: 'flight-screen' }),
      op({ type: 'system-add', id: 'orphan', kind: 'sensors', console: 'missing' }),
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
    await applyShipOperation({ draft, runtime, operation: op({ type: 'system-update', id: 'drive', fields: { kind: 'helm_thrust', console: 'flight-screen', ai_only: false } }) });
    expect(draft.snapshot().history.undo).toHaveLength(1);
    draft.undo(); expect(draft.read(operationPath())).toBe(source);
    await expect(applyShipOperation({ draft, runtime, operation: op({ type: 'system-update', id: 'drive', fields: { kind: 'helm_thrust' } }), current: () => false }))
      .rejects.toThrow('stale-ship-operation');
  });
});

function operationPath() { return 'assets/entities/ship.toml'; }

it('materialises an inherited finite turret as one validated undoable System override', async () => {
  const inheritedSource = source.replace('kind = "helm_thrust"', 'kind = "phaser_bank"\npower_group = "bank"');
  const child = 'tags=["ship"]\nincludes=["base.toml"]\n# child remains\n';
  const draft = new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: manifest }, { path: operationPath(), text: child }]));
  const dependencies = { base_files: { 'assets/entities/base.toml': inheritedSource }, packs: [] };
  const runtime = { shipSchema: async () => ({ ...schema, system_kinds: [...schema.system_kinds, 'phaser_bank'] }), validate: vi.fn(async () => ({ accepted: true })) };
  const operation = op({ type: 'system-composition-set', id: 'drive', components: initialTurret(), channels: [] });
  await expect(applyShipOperation({ draft, dependencies, runtime, operation })).rejects.toThrow(/Materialise/);
  expect(draft.read(operationPath())).toBe(child);
  await applyShipOperation({ draft, dependencies, runtime, operation: { ...operation, materialise: true } });
  const accepted = draft.read(operationPath());
  const actual = inspectShipAuthoring(draft, dependencies, operationPath()).systems.find(row => row.id === 'drive');
  expect(actual).toMatchObject({ console: 'flight-screen', power_group: 'bank', unknown_system: true, component: initialTurret() });
  expect(accepted).toContain('# child remains');
  expect(dependencies.base_files['assets/entities/base.toml']).toBe(inheritedSource);
  expect(draft.snapshot().history.undo).toHaveLength(1);
  draft.undo(); expect(draft.read(operationPath())).toBe(child); draft.redo(); expect(draft.read(operationPath())).toBe(accepted);
  const invalid = initialTurret(); invalid[1].config.traverse_deg_per_sec = 0;
  await expect(applyShipOperation({ draft, dependencies, runtime, operation: { ...operation, components: invalid, materialise: true } })).rejects.toThrow(/finite turret/);
  expect(draft.read(operationPath())).toBe(accepted);
  runtime.validate.mockResolvedValueOnce({ accepted: false, findings: [{ message: 'unsupported actual hardware' }] });
  await expect(applyShipOperation({ draft, dependencies, runtime, operation: { ...operation, components: [], materialise: true } })).rejects.toThrow();
  expect(draft.read(operationPath())).toBe(accepted);
});


it('creates and updates a complete composition through one accepted history entry', async () => {
  const draft = pack();
  const composedSchema = { ...schema, system_kinds: [...schema.system_kinds, 'dock'] };
  const runtime = { dependencies: async () => dependencies, shipSchema: async () => composedSchema,
    validate: vi.fn(async () => ({ accepted: true, findings: [] })) };
  const components = [{ id: 'power', kind: 'power_input' }, { id: 'mate', kind: 'dock_mating', enabled_channel: 'engage', target_channel: 'berth', power_input: 'power',
    config: { range: 100, engage_distance: 200, approach_speed: 10, mate_tolerance: 2, undock_clear_distance: 20, min_power_level: 1 } }];
  const channels = [{ name: 'engage', kind: 'boolean' }, { name: 'berth', kind: 'entity_target' }];
  await applyShipOperation({ draft, runtime, operation: op({ type: 'system-add', id: 'clamps', kind: 'dock', console: 'flight-screen', power_group: 'helm', components, channels }) });
  const created = draft.read(operationPath());
  expect(parse(created).system.at(-1)).toMatchObject({ component: components, channel: channels, power_group: 'helm' });
  const changed = structuredClone(components); changed[1].config.range = 150;
  await applyShipOperation({ draft, runtime, operation: op({ type: 'system-composition-set', id: 'clamps', components: changed, channels }) });
  const edited = draft.read(operationPath());
  expect(parse(edited).system.at(-1).component[1].config.range).toBe(150);
  draft.undo(); expect(draft.read(operationPath())).toBe(created);
  draft.redo(); expect(draft.read(operationPath())).toBe(edited);
  runtime.validate.mockResolvedValueOnce({ accepted: false, findings: [{ severity: 'error', category: 'composition', file: operationPath(), message: 'missing Dock power input' }] });
  await expect(applyShipOperation({ draft, runtime, operation: op({ type: 'system-composition-set', id: 'clamps', components: [changed[1]], channels }) })).rejects.toThrow();
  expect(draft.read(operationPath())).toBe(edited);
});

it.each(['\n', '\r\n'])('keeps System ownership scalars outside nested AI tables (%j)', nl => {
  const draft = pack();
  const text = source + `
[system.config.ai]
root = "idle"
[[system.config.ai.tree]]
id = "idle"
initial = "wait"
[[system.config.ai.tree.state]]
id = "wait"
`;
  draft.edit(operationPath(), text.replace(/\r?\n/g, nl));
  const before = parse(draft.read(operationPath())).system.at(-1).config.ai;
  const update = prepareShipOperation(draft, dependencies, schema, op({ type: 'system-update', id: 'sensors',
    fields: { console: 'flight-screen', ai_only: false, power_group: 'helm' } }));
  const updated = parse(update.changes[0].after).system.at(-1);
  expect(updated).toMatchObject({ console: 'flight-screen', ai_only: false, power_group: 'helm' });
  expect(updated.config.ai).toEqual(before);
  draft.apply(update.changes);
  const cleared = prepareShipOperation(draft, dependencies, schema, op({ type: 'system-update', id: 'sensors',
    fields: { power_group: undefined } }));
  const final = parse(cleared.changes[0].after).system.at(-1);
  expect(final).not.toHaveProperty('power_group');
  expect(final.config.ai).toEqual(before);
});

it('rebinds both illumination AI output meanings when a destination collides with temporary rename names',async()=>{
 const draft=pack(),initial=initialIllumination(),data=parse(source);
 data.system.push({id:'science-light',kind:'illumination',console:'scan-screen',power_group:'science',component:initial.components,channel:initial.channels,
 config:{ai:{root:'support',tree:[{id:'support',initial:'watch',state:[{id:'watch',task:[{id:'enable',action:{kind:'write',channel:'enabled',value:{source:'literal',value:true}}},{id:'target',action:{kind:'write',channel:'target',value:{source:'literal',value:false}}}]}]}]}}});
 draft.edit(op({}).path,stringify(data));const candidate=structuredClone(initial);
 candidate.channels[0].name='workshop-rebind-1';candidate.channels[1].name='subject';candidate.components[1].enabled_channel='workshop-rebind-1';candidate.components[1].target_channel='subject';
 const runtime={shipSchema:async()=>({...schema,system_kinds:[...schema.system_kinds,'illumination']}),dependencies:async()=>dependencies,validate:vi.fn(async()=>({accepted:true,findings:[]}))};
 await applyShipOperation({draft,runtime,operation:op({type:'system-composition-set',id:'science-light',...candidate})});
 const actual=parse(draft.read(op({}).path)).system.find(row=>row.id==='science-light');
 expect(actual.config.ai.tree.find(tree=>tree.id==='support'&&!tree._remove).state[0].task.map(task=>task.action.channel)).toEqual(['workshop-rebind-1','subject']);
 expect(actual.component[1]).toMatchObject({enabled_channel:'workshop-rebind-1',target_channel:'subject'});
});

it('rebinds both jamming AI output meanings when a destination collides with temporary rename names',async()=>{
 const draft=pack(),initial=initialJamming(),data=parse(source);
 data.system.push({id:'comms-jam',kind:'jamming',console:'scan-screen',power_group:'radio',component:initial.components,channel:initial.channels,
 config:{ai:{root:'support',tree:[{id:'support',initial:'watch',state:[{id:'watch',task:[{id:'enable',action:{kind:'write',channel:'enabled',value:{source:'literal',value:true}}},{id:'target',action:{kind:'write',channel:'target',value:{source:'literal',value:false}}}]}]}]}}});
 draft.edit(op({}).path,stringify(data));const candidate=structuredClone(initial);
 candidate.channels[0].name='workshop-rebind-1';candidate.channels[1].name='subject';candidate.components[1].enabled_channel='workshop-rebind-1';candidate.components[1].target_channel='subject';
 const runtime={shipSchema:async()=>({...schema,system_kinds:[...schema.system_kinds,'jamming']}),dependencies:async()=>dependencies,validate:vi.fn(async()=>({accepted:true,findings:[]}))};
 await applyShipOperation({draft,runtime,operation:op({type:'system-composition-set',id:'comms-jam',...candidate})});
 const actual=parse(draft.read(op({}).path)).system.find(row=>row.id==='comms-jam');
 expect(actual.config.ai.tree.find(tree=>tree.id==='support'&&!tree._remove).state[0].task.map(task=>task.action.channel)).toEqual(['workshop-rebind-1','subject']);
 expect(actual.component[1]).toMatchObject({enabled_channel:'workshop-rebind-1',target_channel:'subject'});
});

it('applies one protection composition transaction, retaining initial priorities and renaming only the enabled tree output',async()=>{
 const draft=pack(),data=resolveTemplate('assets/entities/protection_cruiser.toml',{read:path=>fs.existsSync(path)?fs.readFileSync(path,'utf8'):null},parse).resolved.value;
 const owner=data.system.find(row=>row.id==='signal-protection'),candidate={components:structuredClone(owner.component),channels:structuredClone(owner.channel)};
 draft.edit(op({}).path,stringify(data));
 candidate.channels[0].name='active';candidate.channels[1].name='ordered';candidate.components[1].enabled_channel='active';candidate.components[1].priorities_channel='ordered';candidate.channels[1].initial.value=['sensors','comms'];
 const feeds=data.console.flatMap(row=>row.feed||[]);
 const runtime={shipSchema:async()=>({...schema,system_kinds:[...new Set(data.system.map(row=>row.kind))],console_publication_kinds:Object.fromEntries(feeds.map(row=>[row.source,row.kind])),console_feed_kinds:[...new Set(feeds.map(row=>row.kind))],console_stock_publication_kind:'Stock'}),dependencies:async()=>dependencies,validate:vi.fn(async()=>({accepted:true,findings:[]}))};
 const before=draft.read(op({}).path);await applyShipOperation({draft,runtime,operation:op({type:'system-composition-set',id:owner.id,...candidate})});
 const actual=parse(draft.read(op({}).path)).system.find(row=>row.id===owner.id);
 expect(actual.config.ai.tree.find(tree=>!tree._remove).state[0].task[0].action.channel).toBe('active');expect(actual.channel).toEqual(candidate.channels);expect(actual.component).toEqual(candidate.components);
 expect(runtime.validate).toHaveBeenCalledOnce();expect(draft.read(op({}).path)).not.toBe(before);
 const accepted=draft.read(op({}).path),bad=structuredClone(candidate);bad.components[1].config.systems['helm-thrust']=1;
 await expect(applyShipOperation({draft,runtime,operation:op({type:'system-composition-set',id:owner.id,...bad})})).rejects.toThrow(/compatible/);expect(draft.read(op({}).path)).toBe(accepted);expect(runtime.validate).toHaveBeenCalledOnce();
});
