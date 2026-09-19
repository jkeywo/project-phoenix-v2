import { describe, expect, it } from 'vitest';
import { authoredInclude, componentAddable, componentSkeleton, ENTITY_PATH, fieldComponent, fieldGroups,
  fieldIsEditable, fieldIsMaterialisable, fieldIsScalar, fieldSegments, fieldsForm, includeChoices, includesForm,
  moveInclude, newInclude, planComponentAdd, planComponentRemove, planFieldEdits, planIncludeEdits, refusalMessage,
  refusalStringId, templateChoices } from '../workshop-entity.js';

const TEMPLATE = 'assets/entities/mine.toml';
const FRAGMENT = 'assets/entities/fragments/ai/base.toml';
const OTHER = 'assets/entities/fragments/ai/captain.toml';
const BASE_HULL = 'assets/entities/alliance_cruiser.toml';

const composition = (overrides = {}) => ({
  path: TEMPLATE, origin: 'draft', resolvable: true, error: null,
  includes: [{ index: 0, authored: 'fragments/ai/base.toml', canonical: FRAGMENT, line: 4, origin: 'draft' }],
  sources: [FRAGMENT, TEMPLATE],
  components: [
    // The runtime's own shape: the availability flag, and the default's text
    // beside it exactly when the flag is set (entity.rs `ComponentView`).
    { key: 'hull', local: true, local_line: 6, inherited_from: null, skeleton: true,
      skeleton_source: '{ hull_integrity = 100 }' },
    { key: 'ai_profile', local: false, local_line: null, inherited_from: FRAGMENT, skeleton: true,
      skeleton_source: '{ stance = "escort" }' },
    { key: 'comms', local: false, local_line: null, inherited_from: null, skeleton: true,
      skeleton_source: '{ channel = "open" }' },
    { key: 'shape', local: false, local_line: null, inherited_from: null, skeleton: false, skeleton_source: null },
  ],
  fields: [
    { address: 'hull.hull_integrity', source: TEMPLATE, chain: [TEMPLATE], local: true, value_source: '100', line: 7,
      kind: 'integer', materialisable: false },
    { address: 'ai_profile.stance', source: FRAGMENT, chain: [FRAGMENT, TEMPLATE], local: false,
      value_source: '"escort"', line: null, kind: 'string', materialisable: true },
    { address: 'system[id=helm-thrust].ai_only', source: TEMPLATE, chain: [TEMPLATE], local: true, value_source: 'true',
      line: 11, kind: 'boolean', materialisable: false },
    // A local value the exact-source `set` route cannot replace: it is a list,
    // and contract B's field route is scalars only.
    { address: 'tags', source: TEMPLATE, chain: [TEMPLATE], local: true, value_source: '["ship"]', line: 3,
      kind: 'array', materialisable: false },
  ],
  supported_components: ['hull', 'ai_profile', 'comms', 'shape'],
  fragment_choices: [{ path: FRAGMENT, origin: 'draft' }, { path: OTHER, origin: 'base' }],
  findings: [],
  ...overrides,
});

describe('entity include authoring', () => {
  it('writes a chosen fragment as the path the resolver reads: relative to the declaring template', () => {
    expect(authoredInclude(TEMPLATE, FRAGMENT)).toBe('fragments/ai/base.toml');
    expect(authoredInclude(TEMPLATE, 'assets/entities/sibling.toml')).toBe('sibling.toml');
    expect(authoredInclude('assets/entities/fragments/ai/base.toml', 'assets/entities/mine.toml')).toBe('../../mine.toml');
    expect(ENTITY_PATH.test(FRAGMENT)).toBe(true);
    expect(ENTITY_PATH.test('assets/worlds/x.toml')).toBe(false);
  });

  it('plans nothing when the form matches the reading and an insert when a fragment is added', () => {
    const current = composition();
    const form = includesForm(current);
    expect(form.includes).toEqual([{ authored: 'fragments/ai/base.toml', canonical: FRAGMENT, origin: 'draft', line: 4 }]);
    expect(planIncludeEdits({ composition: current, form })).toEqual([]);
    form.includes.push(newInclude(TEMPLATE, { path: OTHER, origin: 'base' }));
    expect(planIncludeEdits({ composition: current, form })).toEqual([
      { op: 'insert', path: ['includes'], index: 1, value_source: '"fragments/ai/captain.toml"' },
    ]);
  });

  it('creates the key as a whole array when the reading has no includes at all', () => {
    const current = composition({ includes: [] });
    const form = includesForm(current);
    form.includes.push(newInclude(TEMPLATE, { path: FRAGMENT, origin: 'draft' }));
    form.includes.push(newInclude(TEMPLATE, { path: OTHER, origin: 'base' }));
    expect(planIncludeEdits({ composition: current, form })).toEqual([
      { op: 'put', path: ['includes'], value_source: '["fragments/ai/base.toml", "fragments/ai/captain.toml"]' },
    ]);
  });

  it('expresses a reorder as set edits in place, never as a remove and an append', () => {
    const current = composition({ includes: [
      { index: 0, authored: 'fragments/ai/base.toml', canonical: FRAGMENT, line: 4, origin: 'draft' },
      { index: 1, authored: 'fragments/ai/captain.toml', canonical: OTHER, line: 5, origin: 'base' },
    ] });
    const form = includesForm(current);
    expect(moveInclude(form, 0, -1)).toBeNull();
    expect(moveInclude(form, 1, 1)).toBeNull();
    expect(moveInclude(form, 0, 1)).toBe(1);
    expect(form.includes.map(entry => entry.authored)).toEqual(['fragments/ai/captain.toml', 'fragments/ai/base.toml']);
    const edits = planIncludeEdits({ composition: current, form });
    expect(edits).toEqual([
      { op: 'set', path: ['includes', 0], value_source: '"fragments/ai/captain.toml"' },
      { op: 'set', path: ['includes', 1], value_source: '"fragments/ai/base.toml"' },
    ]);
    expect(edits.map(edit => edit.op)).not.toContain('remove');
  });

  it('removes a trailing entry by index and rewrites the ones that moved up into its place', () => {
    const current = composition({ includes: [
      { index: 0, authored: 'fragments/ai/base.toml', canonical: FRAGMENT, line: 4, origin: 'draft' },
      { index: 1, authored: 'fragments/ai/captain.toml', canonical: OTHER, line: 5, origin: 'base' },
    ] });
    const form = includesForm(current);
    form.includes.splice(0, 1);
    expect(planIncludeEdits({ composition: current, form })).toEqual([
      { op: 'set', path: ['includes', 0], value_source: '"fragments/ai/captain.toml"' },
      { op: 'remove', path: ['includes', 1] },
    ]);
  });

  it('refuses in the form what it can see itself, with the offending value as the detail', () => {
    const current = composition();
    const plan = includes => planIncludeEdits({ composition: current, form: { includes } });
    expect(() => plan([{ authored: '', canonical: null }])).toThrow('workshop.entity.refused.empty');
    expect(() => plan([{ authored: '../worlds/x.toml', canonical: 'assets/worlds/x.toml' }]))
      .toThrow('workshop.entity.refused.disallowed');
    expect(() => plan([{ authored: 'mine.toml', canonical: TEMPLATE }])).toThrow('workshop.entity.refused.self');
    try {
      plan([{ authored: 'fragments/ai/base.toml', canonical: FRAGMENT }, { authored: './fragments/ai/base.toml', canonical: FRAGMENT }]);
      throw new Error('expected a refusal');
    } catch (error) {
      expect(error.message).toBe('workshop.entity.refused.duplicate');
      expect(error.detail).toBe('./fragments/ai/base.toml');
    }
  });

  it('refuses only a violation the edit ADDS, so an already-broken list can still be repaired', () => {
    // A hand-edited draft whose includes already names a world file: its OTHER
    // entries must still be reorderable, exactly as the runtime's own
    // `introduced` rule allows (entity.rs).
    const current = composition({ includes: [
      { index: 0, authored: '../worlds/x.toml', canonical: 'assets/worlds/x.toml', line: 4, origin: 'draft' },
      { index: 1, authored: 'fragments/ai/base.toml', canonical: FRAGMENT, line: 5, origin: 'draft' },
      { index: 2, authored: 'fragments/ai/captain.toml', canonical: OTHER, line: 6, origin: 'base' },
    ] });
    const form = includesForm(current);
    expect(moveInclude(form, 1, 1)).toBe(2);
    expect(planIncludeEdits({ composition: current, form })).toEqual([
      { op: 'set', path: ['includes', 1], value_source: '"fragments/ai/captain.toml"' },
      { op: 'set', path: ['includes', 2], value_source: '"fragments/ai/base.toml"' },
    ]);
    // Removing the offending row is allowed too.
    const shorter = includesForm(current);
    shorter.includes.splice(0, 1);
    expect(planIncludeEdits({ composition: current, form: shorter }).map(edit => edit.op))
      .toEqual(['set', 'set', 'remove']);
    // …but a SECOND disallowed entry is new, and that one is refused.
    const worse = includesForm(current);
    worse.includes.push({ authored: '../worlds/y.toml', canonical: 'assets/worlds/y.toml' });
    try {
      planIncludeEdits({ composition: current, form: worse });
      throw new Error('expected a refusal');
    } catch (error) {
      expect(error.message).toBe('workshop.entity.refused.disallowed');
      expect(error.detail).toBe('../worlds/y.toml');
    }
  });

  it('offers only the fragments the runtime named that the form does not already list', () => {
    const current = composition();
    expect(includeChoices(current).map(entry => entry.path)).toEqual([OTHER]);
    expect(includeChoices(current, []).map(entry => entry.path)).toEqual([FRAGMENT, OTHER]);
    expect(includeChoices(current, [FRAGMENT, OTHER])).toEqual([]);
  });
});

describe('entity component authoring', () => {
  it('offers Add only for an absent component the runtime has a default for', () => {
    const [hull, inherited, absent, unskeletoned] = composition().components;
    expect(componentSkeleton(hull)).toBe('{ hull_integrity = 100 }');
    expect(componentSkeleton(unskeletoned)).toBeNull();
    expect(componentSkeleton({ key: 'x', skeleton: true, skeleton_source: '{ a = 1 }' })).toBe('{ a = 1 }');
    expect(componentSkeleton({ key: 'x', skeleton: true })).toBeNull();
    expect(componentAddable(absent)).toBe(true);
    expect(componentAddable(hull)).toBe(false);
    expect(componentAddable(inherited)).toBe(false);
    expect(componentAddable(unskeletoned)).toBe(false);
  });

  it('adds a component as ONE put of the runtime\'s own default and refuses what it cannot write', () => {
    const current = composition();
    expect(planComponentAdd({ composition: current, key: 'comms' }))
      .toEqual([{ op: 'put', path: ['comms'], value_source: '{ channel = "open" }' }]);
    expect(() => planComponentAdd({ composition: current, key: 'hull' })).toThrow('workshop.entity.refused.local');
    expect(() => planComponentAdd({ composition: current, key: 'shape' })).toThrow('workshop.entity.refused.no_skeleton');
    expect(() => planComponentAdd({ composition: current, key: 'nonesuch' })).toThrow('workshop.entity.refused.unsupported');
  });

  it('removes a local component and refuses an inherited one with the member that authors it', () => {
    const current = composition();
    expect(planComponentRemove({ composition: current, key: 'hull' })).toEqual([{ op: 'remove', path: ['hull'] }]);
    try {
      planComponentRemove({ composition: current, key: 'ai_profile' });
      throw new Error('expected a refusal');
    } catch (error) {
      expect(error.message).toBe('workshop.entity.refused.inherited');
      expect(error.detail).toBe(FRAGMENT);
    }
    // Nothing authors it, so there is nothing to remove and the reading has moved on.
    expect(() => planComponentRemove({ composition: current, key: 'comms' })).toThrow('workshop.inspector_stale');
  });
});

describe('entity field ownership', () => {
  it('groups the fields by the component that owns them and keeps the catalog\'s order', () => {
    expect(fieldComponent('hull.hull_integrity')).toBe('hull');
    expect(fieldComponent('system[id=helm-thrust].ai_only')).toBe('system');
    expect(fieldComponent('name')).toBe('name');
    expect(fieldGroups(composition()).map(group => [group.key, group.fields.length]))
      .toEqual([['hull', 1], ['ai_profile', 1], ['system', 1], ['tags', 1]]);
  });

  it('splits a provenance address the way the runtime does, quoted keys and all', () => {
    // include_resolve's join_field quotes any key carrying a dot, a bracket, an
    // equals or a space; entity.rs's split_address tracks that, and so must this
    // — a naive split named a key nothing authors and every Apply was refused.
    expect(fieldSegments('"odd.key".child')).toEqual(['odd.key', 'child']);
    expect(fieldSegments('hull."odd key"')).toEqual(['hull', 'odd key']);
    expect(fieldSegments('"a=b"')).toEqual(['a=b']);
    expect(fieldSegments('"unclosed')).toBeNull();
    expect(fieldSegments('a."".b')).toBeNull();
    expect(fieldIsEditable({ local: true, kind: 'integer', address: 'hull."odd key"' })).toBe(true);
    // A quoted key holding a bracket is still a keyed-array address as far as an
    // edit is concerned: the brackets are inside the quotes, so the key is named
    // whole.
    expect(fieldSegments('"odd[key]"')).toEqual(['odd[key]']);
  });

  it('lets a local dotted SCALAR be typed into and holds everything else read-only', () => {
    expect(fieldSegments('hull.hull_integrity')).toEqual(['hull', 'hull_integrity']);
    expect(fieldSegments('system[id=helm-thrust].ai_only')).toBeNull();
    expect(fieldSegments('')).toBeNull();
    const [local, inherited, keyed, structured] = composition().fields;
    expect(fieldIsEditable(local)).toBe(true);
    expect(fieldIsEditable(inherited)).toBe(false);
    expect(fieldIsEditable(keyed)).toBe(false);
    // A list or a sub-table is a source edit: the `set` route refuses both
    // outright, so the row must not offer a box whose every Apply is refused.
    expect(fieldIsEditable(structured)).toBe(false);
    expect(fieldIsScalar(structured)).toBe(false);
    expect(fieldIsScalar({ kind: 'table' })).toBe(false);
    for (const kind of ['string', 'integer', 'float', 'boolean', 'datetime', null]) {
      expect(fieldIsScalar({ kind }), String(kind)).toBe(true);
    }
  });

  it('offers Materialise only where the runtime said the local document can name the address', () => {
    const [local, inherited] = composition().fields;
    expect(fieldIsMaterialisable(local)).toBe(false);
    expect(fieldIsMaterialisable(inherited)).toBe(true);
    // The runtime refuses a field inside a keyed array entry this template does
    // not author, and says so in the catalog rather than leaving a dead button.
    expect(fieldIsMaterialisable({ ...inherited, materialisable: false })).toBe(false);
    // A catalog that carries no flag at all behaves as before.
    expect(fieldIsMaterialisable({ address: 'a.b', local: false })).toBe(true);
    expect(fieldIsMaterialisable(null)).toBe(false);
  });

  it('plans a set for a changed local scalar and refuses an inherited or keyed one', () => {
    const current = composition();
    const form = fieldsForm(current);
    expect(form.values['ai_profile.stance']).toBe('"escort"');
    expect(planFieldEdits({ composition: current, form })).toEqual([]);
    form.values['hull.hull_integrity'] = '140';
    expect(planFieldEdits({ composition: current, form }))
      .toEqual([{ op: 'set', path: ['hull', 'hull_integrity'], value_source: '140' }]);
    form.values['hull.hull_integrity'] = ' ';
    expect(() => planFieldEdits({ composition: current, form })).toThrow('workshop.entity.refused.empty');
    const inheritedForm = fieldsForm(current);
    inheritedForm.values['ai_profile.stance'] = '"picket"';
    try {
      planFieldEdits({ composition: current, form: inheritedForm });
      throw new Error('expected a refusal');
    } catch (error) {
      expect(error.message).toBe('workshop.entity.refused.inherited');
      expect(error.detail).toBe(FRAGMENT);
    }
    const keyedForm = fieldsForm(current);
    keyedForm.values['system[id=helm-thrust].ai_only'] = 'false';
    expect(() => planFieldEdits({ composition: current, form: keyedForm })).toThrow('workshop.entity.refused.keyed');
    const listForm = fieldsForm(current);
    listForm.values.tags = '["ship", "escort"]';
    expect(() => planFieldEdits({ composition: current, form: listForm }))
      .toThrow('workshop.entity.refused.structured');
  });

  it('offers the draft\'s own templates plus the ones the runtime named', () => {
    const paths = ['scenarios.toml', TEMPLATE, FRAGMENT, 'assets/worlds/w.toml'];
    expect(templateChoices(paths)).toEqual([{ path: FRAGMENT, origin: 'draft' }, { path: TEMPLATE, origin: 'draft' }]);
    expect(templateChoices(paths, composition({ fragment_choices: [{ path: BASE_HULL, origin: 'base' }] })))
      .toEqual([{ path: FRAGMENT, origin: 'draft' }, { path: TEMPLATE, origin: 'draft' }, { path: BASE_HULL, origin: 'base' }]);
    // A dependency template that was READ is offered too, with its own origin.
    expect(templateChoices([], composition({ path: BASE_HULL, origin: 'base', fragment_choices: [] })))
      .toEqual([{ path: BASE_HULL, origin: 'base' }]);
  });
});

describe('entity refusal mapping', () => {
  it('maps the runtime\'s own rule prefix to a sentence per category', () => {
    const cases = {
      'include-missing': 'missing', 'include-cycle': 'cycle', 'include-self': 'self',
      'include-disallowed': 'disallowed', 'include-duplicate': 'duplicate', 'component-unsupported': 'unsupported',
      'component-inherited': 'inherited', 'entity-invalid': 'invalid', 'entity-unresolvable': 'missing',
      'materialise-local': 'local', 'materialise-unknown-address': 'address',
      'materialise-keyed-entry': 'keyed',
    };
    for (const [prefix, category] of Object.entries(cases)) {
      expect(refusalStringId(`${prefix}: something`), prefix).toBe(`workshop.entity.refused.${category}`);
    }
    // An address the local document cannot name yet is NOT a missing fragment:
    // the same rule covers an address the resolved template does not carry, one
    // that cannot be written as TOML and one needing two new local tables, and
    // the include-missing sentence is untrue of all three.
    expect(refusalStringId('materialise-unknown-address: "hull.a.b" would need 2 new local tables'))
      .not.toBe('workshop.entity.refused.missing');
  });

  it('falls back to the words of a message that carries no rule prefix', () => {
    expect(refusalStringId('fragments/ai/base.toml would form a cycle: a -> b -> a')).toBe('workshop.entity.refused.cycle');
    expect(refusalStringId('a template cannot include itself')).toBe('workshop.entity.refused.self');
    expect(refusalStringId('unknown field `helm`, expected one of `hull`, `comms`')).toBe('workshop.entity.refused.unsupported');
    expect(refusalStringId('hull.hull_integrity is already local')).toBe('workshop.entity.refused.local');
    expect(refusalStringId('station.rating is reconciled by key')).toBe('workshop.entity.refused.keyed');
    expect(refusalStringId('the composed template does not parse')).toBe('workshop.entity.refused.invalid');
    expect(refusalStringId('assets/entities/x.toml is not in the candidate or its dependencies'))
      .toBe('workshop.entity.refused.missing');
    expect(refusalStringId('an include must name a path under assets/entities/')).toBe('workshop.entity.refused.disallowed');
    expect(refusalStringId('that fragment is already included')).toBe('workshop.entity.refused.duplicate');
    expect(refusalStringId('inherited from a fragment')).toBe('workshop.entity.refused.inherited');
  });

  it('sends a document-level refusal to the shared stale sentence and keeps unmapped words in the catch-all', () => {
    expect(refusalStringId('unknown-document: assets/entities/x.toml')).toBe('workshop.inspector_stale');
    expect(refusalStringId('The document changed; inspect it again before applying this edit.')).toBe('workshop.inspector_stale');
    // The catch-all is the one row that carries the runtime's own explanation.
    expect(refusalStringId('the runtime is on fire')).toBe('workshop.entity.refused.other');
    expect(refusalStringId('the runtime is on fire', 'workshop.inspector_refused')).toBe('workshop.inspector_refused');
    // wasm-bindgen may reject with a bare string rather than an Error.
    expect(refusalMessage('include-cycle: a -> b')).toBe('include-cycle: a -> b');
    expect(refusalMessage(new Error('include-cycle: a -> b'))).toBe('include-cycle: a -> b');
    expect(refusalMessage(undefined)).toBe('');
  });
});
