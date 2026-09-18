import { describe, expect, it } from 'vitest';
import {
  WORLD_PATH, worldSlugPath, worldTitle, rootsForm, newRoot, moveRoot, offeredShips, planScenarioEdits, worldForm,
  extraWorldChoices, planExtraWorldEdits, refusalStringId, refusalMessage,
} from '../workshop-composition.js';

const WORLD = 'assets/worlds/workshop.toml', CHILD = 'assets/worlds/child.toml', BASE = 'assets/worlds/base.toml';
const HULL = 'assets/entities/hull.toml', CRUISER = 'assets/entities/cruiser.toml', GONE = 'assets/entities/gone.toml';
const manifest = (extra = {}) => ({ path: 'scenarios.toml', kind: 'pack', pack: { id: 'p', name: 'P', version: '1', line: 1 }, content: null,
  scenarios: [
    { index: 0, id: 'first', id_line: 8, world: WORLD, world_line: 9, world_origin: 'draft', label: null,
      ships: [{ path: HULL, line: 10, offered: true }], offered_ships: [HULL, CRUISER] },
    { index: 1, id: 'second', id_line: 12, world: BASE, world_line: 13, world_origin: 'base', label: 'world.second.label', ships: [],
      offered_ships: [] },
    { index: 2, id: 'third', id_line: 15, world: WORLD, world_line: 16, world_origin: 'draft', label: null,
      ships: [{ path: HULL, line: 17, offered: true }, { path: GONE, line: 17, offered: false }], offered_ships: [HULL, CRUISER] },
  ], unknown_keys: [], ...extra });
const world = (extra = {}) => ({ path: WORLD, origin: 'draft', title: 'Workshop', line: 1,
  extra_worlds: [{ index: 0, path: CHILD, line: 3, origin: 'draft' }], script_refs: [],
  available_ships: [{ template_path: HULL, line: 5, origin: 'draft' }, { template_path: CRUISER, line: 8, origin: 'draft' }], ...extra });
const catalog = () => ({
  worlds: [world(), { path: CHILD, origin: 'draft', title: null, line: 1, extra_worlds: [], script_refs: [], available_ships: [] },
    { path: BASE, origin: 'base', title: 'Base', line: 1, extra_worlds: [], script_refs: [], available_ships: [] }],
  choices: { worlds: [{ path: WORLD, origin: 'draft' }, { path: CHILD, origin: 'draft' }, { path: BASE, origin: 'base' }], templates: [] },
});
const ops = edits => edits.map(edit => edit.op);

describe('Workshop world paths and titles', () => {
  it('names a world file from its title under assets/worlds/ and refuses a title with nothing usable in it', () => {
    expect(worldSlugPath('Harrow Reach')).toBe('assets/worlds/harrow-reach.toml');
    expect(worldSlugPath('  The Void_2  ')).toBe('assets/worlds/the-void_2.toml');
    expect(() => worldSlugPath('!!!')).toThrow('workshop.composition.invalid_title');
    expect(() => worldSlugPath('')).toThrow('workshop.composition.invalid_title');
    for (const path of [WORLD, 'assets/worlds/a-b_c.toml']) expect(WORLD_PATH.test(path)).toBe(true);
    for (const path of ['assets/entities/hull.toml', 'assets/worlds/nested/x.toml', 'assets/worlds/x.rhai', 'worlds/x.toml', '']) {
      expect(WORLD_PATH.test(path)).toBe(false);
    }
  });

  it('shows a world by its title, else by its file stem', () => {
    expect(worldTitle(world())).toBe('Workshop');
    expect(worldTitle({ path: CHILD, title: null })).toBe('child');
    expect(worldTitle({ path: CHILD, title: '  ' })).toBe('child');
    expect(worldTitle(null)).toBe('');
  });
});

describe('Roots form', () => {
  it('reads the manifest roots into form values and moves live roots past removed ones', () => {
    const form = rootsForm(manifest());
    expect(form).toEqual([
      { index: 0, id: 'first', world: WORLD, label: '', ships: [HULL], removed: false },
      { index: 1, id: 'second', world: BASE, label: 'world.second.label', ships: [], removed: false },
      { index: 2, id: 'third', world: WORLD, label: '', ships: [HULL, GONE], removed: false },
    ]);
    expect(newRoot(' fourth ', CHILD)).toEqual({ index: null, id: 'fourth', world: CHILD, label: '', ships: [], removed: false });
    expect(moveRoot(form, 0, -1)).toBeNull();
    expect(moveRoot(form, 2, 1)).toBeNull();
    form[1].removed = true;
    // The neighbour is the nearest LIVE root, so the removed one is skipped.
    expect(moveRoot(form, 0, 1)).toBe(2);
    expect(form.map(entry => entry.id)).toEqual(['third', 'second', 'first']);
    expect(moveRoot(form, 1, 1)).toBeNull(); // removed roots do not move
    expect(rootsForm(null)).toEqual([]);
  });

  it('offers the ships a root world declares, falling back to the reading for the world it was read with', () => {
    expect(offeredShips(catalog(), WORLD)).toEqual([HULL, CRUISER]);
    expect(offeredShips(catalog(), CHILD)).toEqual([]);
    const scenario = manifest().scenarios[0];
    expect(offeredShips({ worlds: [] }, WORLD, scenario)).toEqual([HULL, CRUISER]);
    expect(offeredShips({ worlds: [] }, BASE, scenario)).toEqual([]);
    expect(offeredShips(null, WORLD)).toEqual([]);
  });
});

describe('planScenarioEdits', () => {
  it('plans nothing for an unchanged form and refuses what the form can see itself', () => {
    const current = manifest();
    expect(planScenarioEdits({ manifest: current, form: rootsForm(current) })).toEqual([]);
    const blank = rootsForm(current); blank[0].id = ' ';
    expect(() => planScenarioEdits({ manifest: current, form: blank })).toThrow('workshop.composition.refused.empty');
    const noWorld = rootsForm(current); noWorld[0].world = '';
    expect(() => planScenarioEdits({ manifest: current, form: noWorld })).toThrow('workshop.composition.refused.empty');
    const elsewhere = rootsForm(current); elsewhere[0].world = 'assets/entities/hull.toml';
    let refused;
    try { planScenarioEdits({ manifest: current, form: elsewhere }); } catch (error) { refused = error; }
    expect(refused.message).toBe('workshop.composition.refused.disallowed');
    expect(refused.detail).toBe('assets/entities/hull.toml');
    const twice = rootsForm(current); twice[1].id = 'first';
    try { planScenarioEdits({ manifest: current, form: twice }); } catch (error) { refused = error; }
    expect(refused.message).toBe('workshop.composition.refused.duplicate');
    expect(refused.detail).toBe('first');
    // A root the reading no longer has is a stale form, not an edit at that slot.
    const stale = rootsForm(current); stale[0].index = 9;
    expect(() => planScenarioEdits({ manifest: current, form: stale })).toThrow('workshop.inspector_stale');
  });

  it('sets, puts and removes the scalar keys and edits the ships list in place', () => {
    const current = manifest();
    const form = rootsForm(current);
    form[0].id = 'first-renamed'; form[0].label = 'world.first.label'; form[0].ships = [CRUISER, HULL];
    form[1].world = CHILD; form[1].label = ''; form[1].ships = [HULL];
    form[2].ships = [HULL];
    expect(planScenarioEdits({ manifest: current, form })).toEqual([
      { op: 'set', path: ['scenario', 0, 'id'], value_source: '"first-renamed"' },
      { op: 'put', path: ['scenario', 0, 'label'], value_source: '"world.first.label"' },
      { op: 'insert', path: ['scenario', 0, 'ships'], index: 1, value_source: `"${CRUISER}"` },
      { op: 'set', path: ['scenario', 1, 'world'], value_source: `"${CHILD}"` },
      { op: 'remove', path: ['scenario', 1, 'label'] },
      // An empty reading is written whole: the key may be absent.
      { op: 'put', path: ['scenario', 1, 'ships'], value_source: `["${HULL}"]` },
      { op: 'remove', path: ['scenario', 2, 'ships', 1] },
    ]);
    const missingId = manifest(); missingId.scenarios[0].id = null;
    expect(planScenarioEdits({ manifest: missingId, form: rootsForm(current) })).toEqual([
      { op: 'put', path: ['scenario', 0, 'id'], value_source: '"first"' },
    ]);
  });

  it('expresses a reorder as set edits on the slots whose content moved, never as a remove and append', () => {
    const current = manifest();
    const form = rootsForm(current);
    expect(moveRoot(form, 0, 1)).toBe(1);
    const edits = planScenarioEdits({ manifest: current, form });
    expect(edits).toEqual([
      { op: 'set', path: ['scenario', 0, 'id'], value_source: '"second"' },
      { op: 'set', path: ['scenario', 0, 'world'], value_source: `"${BASE}"` },
      { op: 'put', path: ['scenario', 0, 'label'], value_source: '"world.second.label"' },
      { op: 'remove', path: ['scenario', 0, 'ships', 0] },
      { op: 'set', path: ['scenario', 1, 'id'], value_source: '"first"' },
      { op: 'set', path: ['scenario', 1, 'world'], value_source: `"${WORLD}"` },
      { op: 'remove', path: ['scenario', 1, 'label'] },
      { op: 'put', path: ['scenario', 1, 'ships'], value_source: `["${HULL}"]` },
    ]);
    expect(ops(edits)).not.toContain('append_table');
    // Moving back leaves nothing to write.
    expect(moveRoot(form, 1, -1)).toBe(0);
    expect(planScenarioEdits({ manifest: current, form })).toEqual([]);
  });

  it('fills the surviving slots in form order, removes by descending index and appends new roots last', () => {
    const current = manifest();
    const form = rootsForm(current);
    form[1].removed = true;
    expect(moveRoot(form, 2, -1)).toBe(0);
    form.push(newRoot('fourth', CHILD));
    const added = newRoot('fifth', WORLD); added.label = 'world.fifth.label'; added.ships = [CRUISER];
    form.push(added);
    expect(planScenarioEdits({ manifest: current, form })).toEqual([
      // Slot 0 now holds the third root: its id and its second ship arrive.
      { op: 'set', path: ['scenario', 0, 'id'], value_source: '"third"' },
      { op: 'insert', path: ['scenario', 0, 'ships'], index: 1, value_source: `"${GONE}"` },
      // Slot 2 now holds the first root.
      { op: 'set', path: ['scenario', 2, 'id'], value_source: '"first"' },
      { op: 'remove', path: ['scenario', 2, 'ships', 1] },
      { op: 'remove', path: ['scenario', 1] },
      { op: 'append_table', path: ['scenario'], fields: [['id', '"fourth"'], ['world', `"${CHILD}"`]] },
      { op: 'append_table', path: ['scenario'], fields: [['id', '"fifth"'], ['world', `"${WORLD}"`], ['label', '"world.fifth.label"'],
        ['ships', `["${CRUISER}"]`]] },
    ]);
    // Removing everything and adding one: three removals, high to low, then the append.
    const cleared = rootsForm(current).map(entry => ({ ...entry, removed: true }));
    cleared.push(newRoot('only', WORLD));
    expect(planScenarioEdits({ manifest: current, form: cleared })).toEqual([
      { op: 'remove', path: ['scenario', 2] }, { op: 'remove', path: ['scenario', 1] }, { op: 'remove', path: ['scenario', 0] },
      { op: 'append_table', path: ['scenario'], fields: [['id', '"only"'], ['world', `"${WORLD}"`]] },
    ]);
    // A pending root that is dropped again leaves no trace.
    const dropped = rootsForm(current); dropped.push(newRoot('gone', WORLD)); dropped.pop();
    expect(planScenarioEdits({ manifest: current, form: dropped })).toEqual([]);
  });
});

describe('planExtraWorldEdits', () => {
  it('inserts, removes and materialises the extra_worlds list, and offers only worlds not yet listed', () => {
    const current = world();
    expect(worldForm(current)).toEqual({ extra_worlds: [CHILD] });
    expect(planExtraWorldEdits({ world: current, form: worldForm(current) })).toEqual([]);
    expect(planExtraWorldEdits({ world: current, form: { extra_worlds: [CHILD, BASE] } })).toEqual([
      { op: 'insert', path: ['extra_worlds'], index: 1, value_source: `"${BASE}"` },
    ]);
    expect(planExtraWorldEdits({ world: current, form: { extra_worlds: [] } })).toEqual([
      { op: 'remove', path: ['extra_worlds', 0] },
    ]);
    expect(planExtraWorldEdits({ world: current, form: { extra_worlds: [BASE] } })).toEqual([
      { op: 'remove', path: ['extra_worlds', 0] },
      { op: 'insert', path: ['extra_worlds'], index: 0, value_source: `"${BASE}"` },
    ]);
    const bare = world({ extra_worlds: [] });
    expect(planExtraWorldEdits({ world: bare, form: { extra_worlds: [CHILD, BASE] } })).toEqual([
      { op: 'put', path: ['extra_worlds'], value_source: `["${CHILD}", "${BASE}"]` },
    ]);
    expect(extraWorldChoices(catalog().choices, current).map(entry => entry.path)).toEqual([BASE]);
    expect(extraWorldChoices(catalog().choices, current, []).map(entry => entry.path)).toEqual([CHILD, BASE]);
    expect(extraWorldChoices({}, current)).toEqual([]);
  });

  it('refuses a world listing itself, listing one twice, or naming a path outside assets/worlds/', () => {
    const current = world();
    const refusal = form => { try { planExtraWorldEdits({ world: current, form }); } catch (error) { return error; } return null; };
    expect(refusal({ extra_worlds: [CHILD, WORLD] })).toMatchObject({ message: 'workshop.composition.refused.self', detail: WORLD });
    expect(refusal({ extra_worlds: [CHILD, CHILD] })).toMatchObject({ message: 'workshop.composition.refused.duplicate', detail: CHILD });
    expect(refusal({ extra_worlds: ['assets/entities/hull.toml'] }))
      .toMatchObject({ message: 'workshop.composition.refused.disallowed', detail: 'assets/entities/hull.toml' });
  });
});

describe('refusalStringId', () => {
  it('maps the runtime refusal by the rule it opens with, as composition.rs spells it', () => {
    // `<rule>: <detail>` — the rule is the finding category the same violation
    // reports as; the detail is the runtime's own sentence about the value.
    const cases = {
      'duplicate-scenario-id: scenario id "one" is declared more than once': 'duplicate',
      'invalid-manifest-entry: scenario[0] has an empty id': 'empty',
      'invalid-manifest-entry: scenario[1] has an empty world path': 'empty',
      'scenario-world-disallowed: scenario[0] world "assets/entities/hull.toml" is not an assets/worlds/*.toml path': 'disallowed',
      'missing-scenario-world: scenario[0] world "assets/worlds/nope.toml" is not in the draft or its dependencies': 'missing',
      'unknown-scenario-ship: scenario[0] curates ship "assets/entities/ghost.toml" which world "assets/worlds/root.toml" does not offer': 'ship',
      'manifest-header-removed: the [pack] table must stay': 'header',
      'extra-worlds-disallowed: extra_worlds[0] "assets/entities/hull.toml" is not an assets/worlds/*.toml path': 'disallowed',
      'extra-worlds-self: extra_worlds[0] names the world itself': 'self',
      'extra-worlds-duplicate: extra_worlds[1] "assets/worlds/child.toml" is already listed': 'duplicate',
      'extra-worlds-missing: extra_worlds[0] "assets/worlds/nope.toml" is not in the draft or its dependencies': 'missing',
      'extra-worlds-cycle: extra_worlds[0] "assets/worlds/loop.toml" composes a cycle: assets/worlds/root.toml -> assets/worlds/loop.toml -> assets/worlds/root.toml': 'cycle',
    };
    for (const [message, category] of Object.entries(cases)) {
      expect(refusalStringId(message), message).toBe(`workshop.composition.refused.${category}`);
    }
    // The prefix decides even when the detail carries another rule's words.
    expect(refusalStringId('extra-worlds-missing: extra_worlds[0] "assets/worlds/cycle.toml" is not in the draft or its dependencies'))
      .toBe('workshop.composition.refused.missing');
    // A refusal of the DOCUMENT is the shared stale sentence, whether or not it
    // carries a prefix: the exact-source owner's own words for a stale
    // `expected_source`, and `unknown-document`, both say the panel's reading has
    // parted from the draft. Neither may borrow a composition rule's words — the
    // missing-world sentence would tell the author something false — and "inspect
    // the current source" is the one thing that answers both.
    expect(refusalStringId('The document changed; inspect it again before applying this edit.')).toBe('workshop.inspector_stale');
    expect(refusalStringId('unknown-document: "assets/worlds/elsewhere.toml" is not a draft member')).toBe('workshop.inspector_stale');
  });

  it('falls back to the words a rule has to carry when the message has no rule prefix', () => {
    const cases = {
      'extra_worlds entry assets/worlds/b.toml would form a cycle: a -> b -> a': 'cycle',
      'extra_worlds entry assets/worlds/a.toml names the world itself': 'self',
      'duplicate scenario id "one"': 'duplicate',
      'extra_worlds lists assets/worlds/b.toml twice': 'duplicate',
      'ships entry assets/entities/x.toml is not offered by assets/worlds/a.toml': 'ship',
      'the [pack] header was removed': 'header',
      'scenario id is empty': 'empty',
      'world assets/worlds/x.toml is missing from the candidate and its dependencies': 'missing',
      'extra_worlds entry assets/worlds/x.toml is not in the candidate or its dependencies': 'missing',
      'world assets/entities/x.toml is not an assets/worlds/*.toml path': 'disallowed',
      'extra_worlds entry lib/x.toml is not allowed': 'disallowed',
    };
    for (const [message, category] of Object.entries(cases)) {
      expect(refusalStringId(message), message).toBe(`workshop.composition.refused.${category}`);
    }
    // An unmapped refusal keeps the runtime's own sentence, which is all the
    // author has when the panel recognises no rule in it.
    expect(refusalStringId('something else entirely')).toBe('workshop.composition.refused.other');
    expect(refusalStringId('', 'workshop.native_save_refused')).toBe('workshop.native_save_refused');
    // wasm-bindgen may reject with a string rather than an Error.
    expect(refusalStringId(new Error('would form a cycle'))).toBe('workshop.composition.refused.cycle');
    expect(refusalMessage('plain')).toBe('plain');
    expect(refusalMessage(new Error('boxed'))).toBe('boxed');
    expect(refusalMessage(null)).toBe('');
  });
});
