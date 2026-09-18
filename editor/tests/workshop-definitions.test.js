import { describe, expect, it } from 'vitest';
import { WorkshopDocument } from '../workshop-document.js';
import { createStoreZip } from '../mod-pack-export.js';
import {
  textMembers, definitionsSnapshot, snapshotIsCurrent, factionSlug, factionSlugPath, enemyChoices,
  tomlBasicString, decodeTomlString, complianceForm, factionForm, planFactionEdits, ratingForm, planRatingEdits,
  newRung, rungNames, findingsAt, draftFirst,
} from '../workshop-definitions.js';

const A = 'aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa';
const B = 'bbbbbbbb-2222-4222-8222-bbbbbbbbbbbb';
const C = 'cccccccc-3333-4333-8333-cccccccccccc';
const defaults = { ack_secs: 2, decide_secs: 3, hold: 'comply', divert: 'comply', dock: 'comply' };
const choices = { factions: [{ uuid: A, name: 'Mine', origin: 'draft' }, { uuid: B, name: 'Alliance', origin: 'base' },
  { uuid: C, name: 'Pirate', origin: 'base' }], order_responses: ['comply', 'refuse'], ai_rules: ['torpedo_auto_fire'] };
const faction = (extra = {}) => ({ path: 'assets/factions/mine.toml', origin: 'draft', uuid: A, uuid_line: 1, name: 'Mine', name_line: 2,
  display_name: null, display_name_line: null, enemies: [{ uuid: B, line: 4, name: 'Alliance' }], compliance: null, unknown_keys: [], ...extra });
const station = (extra = {}) => ({ id: 'helm', name: 'Helm', line: 10, human_seeking: true, visiting_rating: { source: '"Std"', line: 12 },
  systems: [{ id: 'helm-lateral-thrust', kind: 'thruster', line: 3 }, { id: 'helm-impulse', kind: 'impulse', line: 5 }],
  ratings: [
    { index: 0, name: 'Std', line: 14, automated_systems: [], ai_tuning: [] },
    { index: 1, name: 'Simplified', line: 17, automated_systems: [{ id: 'helm-lateral-thrust', line: 18, owned: true },
      { id: 'weapons-phaser', line: 18, owned: false }], ai_tuning: [{ rule: 'torpedo_auto_fire', line: 19 }] },
  ], ...extra });

describe('Workshop definition members and readings', () => {
  it('reads only the toml and rhai members and detects any later change to them', () => {
    const draft = new WorkshopDocument(createStoreZip([
      { path: 'scenarios.toml', text: '[pack]\n' }, { path: 'assets/factions/mine.toml', text: 'name = "Mine"\n' },
      { path: 'assets/scripts/a.rhai', text: 'fn f() {}\n' }, { path: 'assets/models/ship.glb', bytes: new Uint8Array([1, 2]) },
    ]), { kind: 'project' });
    expect(Object.keys(textMembers(draft))).toEqual(['scenarios.toml', 'assets/factions/mine.toml', 'assets/scripts/a.rhai']);
    const snapshot = definitionsSnapshot(draft);
    expect(snapshotIsCurrent(snapshot, draft)).toBe(true);
    // A change to a member the catalog did not list still counts: entity and
    // world references are part of what the reading reported.
    draft.edit('assets/scripts/a.rhai', 'fn g() {}\n');
    expect(snapshotIsCurrent(snapshot, draft)).toBe(false);
    draft.undo();
    expect(snapshotIsCurrent(snapshot, draft)).toBe(true);
    draft.put('assets/factions/new.toml', 'name = "New"\n');
    expect(snapshotIsCurrent(snapshot, draft)).toBe(false);
    draft.undo();
    expect(snapshotIsCurrent(snapshot, draft)).toBe(true);
    const other = new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: '[pack]\n' }]), { kind: 'project' });
    expect(snapshotIsCurrent(snapshot, other)).toBe(false);
    expect(snapshotIsCurrent(null, draft)).toBe(false);
  });

  it('names a faction file from its reference name and refuses a name with nothing usable in it', () => {
    expect(factionSlug('Harrow Navy')).toBe('harrow-navy');
    expect(factionSlug("  The Void's Reach  ")).toBe('the-void-s-reach');
    expect(factionSlugPath('Alliance')).toBe('assets/factions/alliance.toml');
    expect(factionSlugPath('Free_Traders-2')).toBe('assets/factions/free_traders-2.toml');
    expect(() => factionSlugPath('!!!')).toThrow('workshop.definitions.invalid_name');
    expect(() => factionSlugPath('')).toThrow('workshop.definitions.invalid_name');
  });

  it('offers as enemies only the factions that are neither itself nor already listed', () => {
    expect(enemyChoices(choices, faction()).map(entry => entry.uuid)).toEqual([C]);
    expect(enemyChoices(choices, faction(), [B, C])).toEqual([]);
    expect(enemyChoices(choices, faction({ enemies: [] })).map(entry => entry.uuid)).toEqual([B, C]);
    expect(enemyChoices({}, faction())).toEqual([]);
  });

  it('orders draft definitions before read-only ones and finds findings by line', () => {
    expect(draftFirst([{ origin: 'base', path: 'b' }, { origin: 'draft', path: 'd' }, { origin: 'pack:x', path: 'p' }])
      .map(entry => entry.path)).toEqual(['d', 'b', 'p']);
    const catalog = { findings: [{ file: 'a.toml', line: 3, message: 'one' }, { file: 'a.toml', line: 4, message: 'two' },
      { file: 'b.toml', line: 3, message: 'three' }] };
    expect(findingsAt(catalog, 'a.toml', 3).map(finding => finding.message)).toEqual(['one']);
    expect(findingsAt(catalog, 'a.toml', undefined)).toEqual([]);
  });
});

describe('TOML scalar text', () => {
  it('quotes a basic string exactly and reads basic and literal strings back', () => {
    expect(tomlBasicString('plain')).toBe('"plain"');
    expect(tomlBasicString('say "hi"\\now\n')).toBe('"say \\"hi\\"\\\\now\\n"');
    expect(tomlBasicString('\u0001')).toBe('"\\u0001"');
    expect(decodeTomlString('"say \\"hi\\"\\\\now\\n"')).toBe('say "hi"\\now\n');
    expect(decodeTomlString("'C:\\raw'")).toBe('C:\\raw');
    expect(decodeTomlString('"\\u00E9"')).toBe('é');
    expect(decodeTomlString(' 42 ')).toBe('42');
    expect(decodeTomlString(null)).toBe('');
  });
});

describe('planFactionEdits', () => {
  it('plans nothing for an unchanged form and refuses an empty name', () => {
    const definition = faction();
    expect(planFactionEdits({ definition, form: factionForm(definition, defaults), defaults })).toEqual([]);
    expect(() => planFactionEdits({ definition, form: { ...factionForm(definition), name: '  ' } })).toThrow('workshop.definitions.invalid_name');
  });

  it('sets, puts and removes the scalar keys from the reading', () => {
    const definition = faction();
    const form = { ...factionForm(definition, defaults), name: 'Mine Two', display_name: 'faction.mine.display_name' };
    expect(planFactionEdits({ definition, form, defaults })).toEqual([
      { op: 'set', path: ['name'], value_source: '"Mine Two"' },
      { op: 'put', path: ['display_name'], value_source: '"faction.mine.display_name"' },
    ]);
    const named = faction({ display_name: 'faction.mine.display_name', display_name_line: 3 });
    expect(planFactionEdits({ definition: named, form: { ...factionForm(named), display_name: '' } })).toEqual([
      { op: 'remove', path: ['display_name'] },
    ]);
    expect(planFactionEdits({ definition: faction({ name: null }), form: { ...factionForm(faction()), name: 'Mine' } })).toEqual([
      { op: 'put', path: ['name'], value_source: '"Mine"' },
    ]);
  });

  it('adds an enemy as one insert after the kept entries and removes by descending index', () => {
    const definition = faction();
    expect(planFactionEdits({ definition, form: { ...factionForm(definition), enemies: [B, C] } })).toEqual([
      { op: 'insert', path: ['enemies'], index: 1, value_source: `"${C}"` },
    ]);
    const three = faction({ enemies: [{ uuid: B, line: 4, name: 'Alliance' }, { uuid: C, line: 5, name: 'Pirate' },
      { uuid: A, line: 6, name: 'Mine' }] });
    expect(planFactionEdits({ definition: three, form: { ...factionForm(three), enemies: [C, B] } })).toEqual([
      { op: 'remove', path: ['enemies', 2] },
    ]);
    expect(planFactionEdits({ definition: three, form: { ...factionForm(three), enemies: [] } })).toEqual([
      { op: 'remove', path: ['enemies', 2] }, { op: 'remove', path: ['enemies', 1] }, { op: 'remove', path: ['enemies', 0] },
    ]);
    // Removing the first and adding another: the insert index counts what is kept.
    expect(planFactionEdits({ definition: three, form: { ...factionForm(three), enemies: [C, A, 'dddddddd-4444-4444-8444-dddddddddddd'] } })).toEqual([
      { op: 'remove', path: ['enemies', 0] },
      { op: 'insert', path: ['enemies'], index: 2, value_source: '"dddddddd-4444-4444-8444-dddddddddddd"' },
    ]);
  });

  it('writes the whole list when the reading had no enemies, because the key may be absent', () => {
    const definition = faction({ enemies: [] });
    expect(planFactionEdits({ definition, form: { ...factionForm(definition), enemies: [B, C] } })).toEqual([
      { op: 'put', path: ['enemies'], value_source: `["${B}", "${C}"]` },
    ]);
  });

  it('materialises compliance as one inline table of the runtime defaults, edited values included', () => {
    const definition = faction();
    const form = { ...factionForm(definition, defaults), compliance: complianceForm(null, defaults) };
    expect(form.compliance).toEqual({ ack_secs: '2', decide_secs: '3', hold: 'comply', divert: 'comply', dock: 'comply' });
    // The key set is the catalog's, not a list typed here.
    expect(complianceForm(null, { ack_secs: 2, grace_secs: 9 })).toEqual({ ack_secs: '2', grace_secs: '9' });
    expect(complianceForm({ refusal: { source: '"faction.mine.refusal"', line: 8 } }, { ack_secs: 2 }))
      .toEqual({ ack_secs: '2', refusal: 'faction.mine.refusal' });
    form.compliance.hold = 'refuse';
    expect(planFactionEdits({ definition, form, defaults })).toEqual([
      { op: 'put', path: ['compliance'], value_source: '{ ack_secs = 2, decide_secs = 3, hold = "refuse", divert = "comply", dock = "comply" }' },
    ]);
    form.compliance.ack_secs = '2.5';
    expect(() => planFactionEdits({ definition, form, defaults })).toThrow('workshop.definitions.invalid_number');
  });

  it('edits a present compliance table key by key, putting only what leaves the default', () => {
    // The catalog reports the refusal reason as `refusal`; the document holds `refusal_reason`.
    const definition = faction({ compliance: { ack_secs: { source: '4', line: 6 }, hold: { source: "'refuse'", line: 7 },
      refusal: { source: '"faction.mine.refusal"', line: 8 } } });
    const form = factionForm(definition, defaults);
    expect(form.compliance).toEqual({ ack_secs: '4', decide_secs: '3', hold: 'refuse', divert: 'comply', dock: 'comply',
      refusal: 'faction.mine.refusal' });
    expect(planFactionEdits({ definition, form, defaults })).toEqual([]);
    form.compliance.ack_secs = '5'; form.compliance.decide_secs = '3'; form.compliance.hold = 'comply';
    form.compliance.dock = 'refuse'; form.compliance.refusal = '';
    expect(planFactionEdits({ definition, form, defaults })).toEqual([
      { op: 'set', path: ['compliance', 'ack_secs'], value_source: '5' },
      { op: 'set', path: ['compliance', 'hold'], value_source: '"comply"' },
      { op: 'put', path: ['compliance', 'dock'], value_source: '"refuse"' },
      { op: 'remove', path: ['compliance', 'refusal_reason'] },
    ]);
    form.compliance.refusal = 'faction.mine.declines';
    expect(planFactionEdits({ definition, form, defaults }).at(-1))
      .toEqual({ op: 'set', path: ['compliance', 'refusal_reason'], value_source: '"faction.mine.declines"' });
    const bare = faction({ compliance: { ack_secs: { source: '4', line: 6 } } });
    expect(planFactionEdits({ definition: bare, form: { ...factionForm(bare, defaults),
      compliance: { ...complianceForm(bare.compliance, defaults), refusal: 'faction.mine.declines' } }, defaults }))
      .toEqual([{ op: 'put', path: ['compliance', 'refusal_reason'], value_source: '"faction.mine.declines"' }]);
    expect(planFactionEdits({ definition, form: { ...form, compliance: null }, defaults })).toEqual([
      { op: 'remove', path: ['compliance'] },
    ]);
  });
});

describe('planRatingEdits', () => {
  it('plans nothing for an unchanged form and refuses blank or duplicate rung names', () => {
    const target = station();
    expect(planRatingEdits({ station: target, stationIndex: 2, form: ratingForm(target) })).toEqual([]);
    const blank = ratingForm(target); blank.ratings[0].name = ' ';
    expect(() => planRatingEdits({ station: target, stationIndex: 2, form: blank })).toThrow('workshop.definitions.invalid_name');
    const twice = ratingForm(target); twice.ratings[0].name = 'Simplified';
    expect(() => planRatingEdits({ station: target, stationIndex: 2, form: twice })).toThrow('workshop.definitions.rung_exists');
    expect(() => planRatingEdits({ station: target, form: ratingForm(target) })).toThrow('workshop.inspector_refused');
    expect(rungNames(twice)).toEqual(['Simplified', 'Simplified']);
  });

  it('sets, puts and removes the visiting rating', () => {
    const target = station();
    const form = ratingForm(target);
    expect(form.visiting_rating).toBe('Std');
    form.visiting_rating = 'Simplified';
    expect(planRatingEdits({ station: target, stationIndex: 0, form })).toEqual([
      { op: 'set', path: ['station', 0, 'visiting_rating'], value_source: '"Simplified"' },
    ]);
    form.visiting_rating = '';
    expect(planRatingEdits({ station: target, stationIndex: 0, form })).toEqual([
      { op: 'remove', path: ['station', 0, 'visiting_rating'] },
    ]);
    const bare = station({ visiting_rating: null });
    expect(planRatingEdits({ station: bare, stationIndex: 0, form: { ...ratingForm(bare), visiting_rating: 'Std' } })).toEqual([
      { op: 'put', path: ['station', 0, 'visiting_rating'], value_source: '"Std"' },
    ]);
  });

  it('renames a rung and inserts and removes its automated systems by index', () => {
    const target = station();
    const form = ratingForm(target);
    form.ratings[1].name = 'Simple';
    form.ratings[1].automated_systems = ['helm-lateral-thrust', 'helm-impulse'];
    expect(planRatingEdits({ station: target, stationIndex: 1, form })).toEqual([
      { op: 'set', path: ['station', 1, 'rating', 1, 'name'], value_source: '"Simple"' },
      { op: 'remove', path: ['station', 1, 'rating', 1, 'automated_systems', 1] },
      { op: 'insert', path: ['station', 1, 'rating', 1, 'automated_systems'], index: 1, value_source: '"helm-impulse"' },
    ]);
    const first = ratingForm(target);
    first.ratings[0].automated_systems = ['helm-impulse'];
    expect(planRatingEdits({ station: target, stationIndex: 1, form: first })).toEqual([
      { op: 'insert', path: ['station', 1, 'rating', 0, 'automated_systems'], index: 0, value_source: '"helm-impulse"' },
    ]);
  });

  it('puts and removes ai_tuning rules one key at a time, whether or not the rung has the table yet', () => {
    const target = station();
    const form = ratingForm(target);
    form.ratings[0].ai_rules = ['torpedo_auto_fire'];
    form.ratings[1].ai_rules = [];
    // Never a whole-table put: a rung whose last rule was removed keeps an
    // empty `[station.rating.ai_tuning]` standard table, which reads as no
    // rules and which the runtime refuses to overwrite with a value; a per-key
    // put enters it and materialises a missing table alike.
    expect(planRatingEdits({ station: target, stationIndex: 0, form })).toEqual([
      { op: 'put', path: ['station', 0, 'rating', 0, 'ai_tuning', 'torpedo_auto_fire'], value_source: '{}' },
      { op: 'remove', path: ['station', 0, 'rating', 1, 'ai_tuning', 'torpedo_auto_fire'] },
    ]);
    const two = ratingForm(target);
    two.ratings[0].ai_rules = ['torpedo_auto_fire', 'frequency_match'];
    expect(planRatingEdits({ station: target, stationIndex: 0, form: two })).toEqual([
      { op: 'put', path: ['station', 0, 'rating', 0, 'ai_tuning', 'torpedo_auto_fire'], value_source: '{}' },
      { op: 'put', path: ['station', 0, 'rating', 0, 'ai_tuning', 'frequency_match'], value_source: '{}' },
    ]);
    const more = ratingForm(target);
    more.ratings[1].ai_rules = ['torpedo_auto_fire', 'frequency_match'];
    expect(planRatingEdits({ station: target, stationIndex: 0, form: more })).toEqual([
      { op: 'put', path: ['station', 0, 'rating', 1, 'ai_tuning', 'frequency_match'], value_source: '{}' },
    ]);
  });

  it('removes rungs by descending index after the scalar edits and appends new rungs last', () => {
    const target = station({ ratings: [...station().ratings, { index: 2, name: 'Expert', line: 21, automated_systems: [], ai_tuning: [] }] });
    const form = ratingForm(target);
    form.ratings[0].removed = true;
    form.ratings[2].removed = true;
    form.ratings[1].name = 'Simple';
    form.ratings.push(newRung('Veteran'));
    const added = newRung('Ace'); added.automated_systems = ['helm-impulse']; added.ai_rules = ['torpedo_auto_fire'];
    form.ratings.push(added);
    expect(rungNames(form)).toEqual(['Simple', 'Veteran', 'Ace']);
    expect(planRatingEdits({ station: target, stationIndex: 3, form })).toEqual([
      { op: 'set', path: ['station', 3, 'rating', 1, 'name'], value_source: '"Simple"' },
      { op: 'remove', path: ['station', 3, 'rating', 2] },
      { op: 'remove', path: ['station', 3, 'rating', 0] },
      { op: 'append_table', path: ['station', 3, 'rating'], fields: [['name', '"Veteran"'], ['automated_systems', '[]']] },
      { op: 'append_table', path: ['station', 3, 'rating'], fields: [['name', '"Ace"'], ['automated_systems', '["helm-impulse"]'],
        ['ai_tuning', '{ torpedo_auto_fire = {} }']] },
    ]);
    // A pending rung that is dropped again leaves no trace.
    const dropped = ratingForm(target);
    dropped.ratings.push(newRung('Gone'));
    dropped.ratings.pop();
    expect(planRatingEdits({ station: target, stationIndex: 3, form: dropped })).toEqual([]);
    // A rung the reading no longer has is a stale form, not an edit at index 9.
    const stale = ratingForm(target);
    stale.ratings[0].index = 9;
    expect(() => planRatingEdits({ station: target, stationIndex: 3, form: stale })).toThrow('workshop.inspector_stale');
  });
});
