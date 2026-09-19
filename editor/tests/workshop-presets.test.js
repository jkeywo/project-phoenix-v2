import { describe, expect, it } from 'vitest';
import { presetsForm, planPresetEdits, movePreset, moveWidget, newWidget, presetMovable, widgetMovable,
  widgetOwns, WIDGET_KEY_OWNERS, WIDGET_KEYS, widgetTypeChoices, widgetActionChoices, contactChoices,
  worldChoices, presetFindings, findingsOn, appendPresetBlock, refusalStringId, refusalMessage,
  RESERVED_PRESET_ID, DRAWN_PANEL_IDS, DRAWN_QUICK_ACTION_IDS } from '../workshop-presets.js';
import { GM_ROLE_PRESET_PANEL_IDS, GM_ROLE_PRESET_QUICK_ACTION_IDS,
  GM_WIDGET_TYPES } from '../../gui/gm-role-presets.js';
import { t } from '../../gui/strings.js';

const WORLD = 'assets/worlds/mine.toml';
const BASE = 'assets/worlds/base.toml';

const widget = (overrides = {}) => ({ index: 0, id: 'load', id_line: 10, kind: 'workload', kind_line: 11,
  label: 'server.gm.load', label_line: 12, band: null, category: null,
  ship: { value: 'Kestrel', line: 13, known: true }, actions: [], text: null, unknown_keys: [], ...overrides });

const catalog = (overrides = {}) => ({
  path: WORLD, origin: 'draft',
  presets: [
    { index: 0, id: 'watch', id_line: 3, label: 'server.gm.watch', label_line: 4,
      panels: [{ index: 0, value: 'gm-map-panel', line: 5 }, { index: 1, value: 'gm-future-panel', line: 5 }],
      quick_actions: [{ index: 0, value: 'gm-session-pause', line: 6 }],
      contacts: [{ index: 0, value: 'Kestrel', line: 7, known: true }],
      widgets: [widget(), widget({ index: 1, id: 'brief', id_line: 16, kind: 'note', kind_line: 17,
        label: 'server.gm.brief', label_line: 18, ship: null, text: { value: 'server.gm.brief.text', line: 19 } })],
      unknown_keys: [] },
    { index: 1, id: 'plain', id_line: 22, label: 'server.gm.plain', label_line: 23,
      panels: [], quick_actions: [], contacts: [], widgets: [], unknown_keys: [] },
  ],
  choices: { widget_types: [...GM_WIDGET_TYPES], widget_actions: ['gm-session-pause', 'gm-session-resume'],
    bands: ['critical', 'elevated', 'steady'], categories: ['damage', 'comms'], entities: ['Kestrel', 'Harrow'] },
  worlds: [{ path: WORLD, origin: 'draft' }, { path: BASE, origin: 'base' }],
  findings: [],
  ...overrides });

const plan = (read, mutate = () => {}) => {
  const form = presetsForm(read);
  mutate(form);
  return planPresetEdits({ catalog: read, form });
};
const refused = (read, mutate) => {
  try { plan(read, mutate); }
  catch (error) { return error; }
  throw new Error('the plan was not refused');
};

describe('Workshop GM role preset authoring', () => {
  it('derives which keys each widget type owns from the browser\'s own normaliser', () => {
    // The ratchet on that derivation: the runtime enforces the same ownership at
    // world load, so a change here is a change to one shared rule and not to a
    // second copy of it.
    expect(WIDGET_KEY_OWNERS).toEqual({
      attention: ['band', 'category', 'ship'],
      workload: ['ship'],
      actions: ['actions'],
      note: ['text'],
    });
    expect(Object.keys(WIDGET_KEY_OWNERS)).toEqual([...GM_WIDGET_TYPES]);
    for (const key of WIDGET_KEYS) expect(widgetOwns('nonexistent', key)).toBe(false);
    expect(widgetOwns('workload', 'band')).toBe(false);
    expect(widgetOwns('note', 'actions')).toBe(false);
    // The browser owns the panel and quick-action vocabularies; this module names
    // them rather than restating them.
    expect(DRAWN_PANEL_IDS).toBe(GM_ROLE_PRESET_PANEL_IDS);
    expect(DRAWN_QUICK_ACTION_IDS).toBe(GM_ROLE_PRESET_QUICK_ACTION_IDS);
    expect(RESERVED_PRESET_ID).toBe('all');
  });

  it('reads every preset with its lines, its facets and which optional widget keys the document carries', () => {
    const form = presetsForm(catalog());
    expect(form.presets[0]).toMatchObject({ index: 0, id: 'watch', id_line: 3, label: 'server.gm.watch',
      label_line: 4, panels: ['gm-map-panel', 'gm-future-panel'], quick_actions: ['gm-session-pause'],
      contacts: ['Kestrel'] });
    expect(form.presets[0].widgets[0]).toMatchObject({ index: 0, id: 'load', kind: 'workload', ship: 'Kestrel',
      band: '', category: '', text: '', actions: [] });
    // Presence is what tells a `put` from a `set`: an absent key is not an empty one.
    expect(form.presets[0].widgets[0].present).toEqual({ band: false, category: false, ship: true,
      actions: false, text: false });
    expect(form.presets[0].widgets[1].present.text).toBe(true);
    expect(plan(catalog())).toEqual([]);
  });

  it('writes a changed id, label and facet list as exact-source edits under their own slots', () => {
    const edits = plan(catalog(), form => {
      form.presets[0].label = 'server.gm.watchful';
      form.presets[0].panels = ['gm-map-panel', 'gm-comms-panel'];
      form.presets[0].contacts = [];
      form.presets[1].quick_actions = ['gm-session-resume'];
    });
    expect(edits).toEqual([
      { op: 'set', path: ['gm_role_preset', 0, 'label'], value_source: '"server.gm.watchful"' },
      { op: 'remove', path: ['gm_role_preset', 0, 'panels', 1] },
      { op: 'insert', path: ['gm_role_preset', 0, 'panels'], index: 1, value_source: '"gm-comms-panel"' },
      { op: 'remove', path: ['gm_role_preset', 0, 'contacts', 0] },
      // An empty reading is written as a whole array: the catalog cannot say
      // whether an empty list is an absent key, and `insert` needs the array.
      { op: 'put', path: ['gm_role_preset', 1, 'quick_actions'], value_source: '["gm-session-resume"]' },
    ]);
  });

  it('removes a whole preset as ONE remove and leaves every survivor in its own slot', () => {
    // The kept presets fill the SURVIVING slots in form order, so a removal alone
    // rewrites nothing: the survivor keeps slot 1 and the array closes up around
    // the removed one. Every other byte of the world is untouched.
    expect(plan(catalog(), form => { form.presets[0].removed = true; }))
      .toEqual([{ op: 'remove', path: ['gm_role_preset', 0] }]);
    expect(plan(catalog(), form => { form.presets[1].removed = true; }))
      .toEqual([{ op: 'remove', path: ['gm_role_preset', 1] }]);
    // Both at once still go by DESCENDING slot, so each remove still names the
    // preset it meant.
    expect(plan(catalog(), form => { form.presets.forEach(entry => { entry.removed = true; }); })).toEqual([
      { op: 'remove', path: ['gm_role_preset', 1] },
      { op: 'remove', path: ['gm_role_preset', 0] },
    ]);
  });

  it('adds a widget as one appended table carrying only the keys its TYPE owns', () => {
    const edits = plan(catalog(), form => {
      const added = newWidget('pressure', 'attention');
      added.band = 'critical'; added.category = 'damage'; added.ship = 'Harrow';
      // A key the type does not own cannot be authored onto it, even when the row
      // object happens to hold one.
      added.text = 'server.gm.note.text'; added.actions = ['gm-session-pause'];
      form.presets[0].widgets.push(added);
    });
    expect(edits).toEqual([{ op: 'append_table', path: ['gm_role_preset', 0, 'widget'], fields: [
      ['id', '"pressure"'], ['type', '"attention"'], ['label', '"pressure"'],
      ['band', '"critical"'], ['category', '"damage"'], ['ship', '"Harrow"'],
    ] }]);
  });

  it('edits and removes a widget under its own slot and never writes a key its type does not own', () => {
    const read = catalog();
    expect(plan(read, form => {
      form.presets[0].widgets[0].ship = 'Harrow';
      form.presets[0].widgets[0].band = 'critical'; // workload owns no band.
      form.presets[0].widgets[1].text = 'server.gm.brief.other';
    })).toEqual([
      { op: 'set', path: ['gm_role_preset', 0, 'widget', 0, 'ship'], value_source: '"Harrow"' },
      { op: 'set', path: ['gm_role_preset', 0, 'widget', 1, 'text'], value_source: '"server.gm.brief.other"' },
    ]);
    // An owned key the document does not carry is a `put`, and clearing one is a
    // `remove` rather than an empty string.
    expect(plan(read, form => { form.presets[0].widgets[1].text = ''; })).toEqual([
      { op: 'remove', path: ['gm_role_preset', 0, 'widget', 1, 'text'] },
    ]);
    expect(plan(read, form => { form.presets[0].widgets[0].ship = ''; })).toEqual([
      { op: 'remove', path: ['gm_role_preset', 0, 'widget', 0, 'ship'] },
    ]);
    // Removing a widget is ONE remove: the survivor keeps its own slot and the
    // array closes up around it, so no other widget's text is rewritten.
    expect(plan(read, form => { form.presets[0].widgets.splice(0, 1); }))
      .toEqual([{ op: 'remove', path: ['gm_role_preset', 0, 'widget', 0] }]);
  });

  it('leaves a key on the WRONG type exactly as the document holds it until the type itself changes', () => {
    // A pre-existing violation is the runtime's finding to report at its line. An
    // edit that does not add one must not quietly remove the offending key — and
    // must not be refused over it either.
    const read = catalog();
    read.presets[0].widgets[0].band = { value: 'critical', line: 14 };
    expect(plan(read, form => { form.presets[0].label = 'server.gm.other'; })).toEqual([
      { op: 'set', path: ['gm_role_preset', 0, 'label'], value_source: '"server.gm.other"' },
    ]);
    // Changing the type is the one case where the old type's keys must go, or the
    // edit would introduce the violation itself.
    expect(plan(read, form => { form.presets[0].widgets[1].kind = 'attention'; })).toEqual([
      { op: 'set', path: ['gm_role_preset', 0, 'widget', 1, 'type'], value_source: '"attention"' },
      { op: 'remove', path: ['gm_role_preset', 0, 'widget', 1, 'text'] },
    ]);
  });

  it('reorders widgets as set edits on the swapped slots and refuses one carrying an unread key', () => {
    const read = catalog();
    const form = presetsForm(read);
    expect(moveWidget(form.presets[0].widgets, 0, 1)).toBe(1);
    expect(form.presets[0].widgets.map(entry => entry.id)).toEqual(['brief', 'load']);
    expect(planPresetEdits({ catalog: read, form })).toEqual([
      { op: 'set', path: ['gm_role_preset', 0, 'widget', 0, 'id'], value_source: '"brief"' },
      { op: 'set', path: ['gm_role_preset', 0, 'widget', 0, 'type'], value_source: '"note"' },
      { op: 'set', path: ['gm_role_preset', 0, 'widget', 0, 'label'], value_source: '"server.gm.brief"' },
      { op: 'remove', path: ['gm_role_preset', 0, 'widget', 0, 'ship'] },
      { op: 'put', path: ['gm_role_preset', 0, 'widget', 0, 'text'], value_source: '"server.gm.brief.text"' },
      { op: 'set', path: ['gm_role_preset', 0, 'widget', 1, 'id'], value_source: '"load"' },
      { op: 'set', path: ['gm_role_preset', 0, 'widget', 1, 'type'], value_source: '"workload"' },
      { op: 'set', path: ['gm_role_preset', 0, 'widget', 1, 'label'], value_source: '"server.gm.load"' },
      { op: 'put', path: ['gm_role_preset', 0, 'widget', 1, 'ship'], value_source: '"Kestrel"' },
      { op: 'remove', path: ['gm_role_preset', 0, 'widget', 1, 'text'] },
    ]);
    // An unknown key's VALUE is not in the catalog, so a swap would silently drop
    // it: the move is refused rather than losing authored text.
    const unread = catalog();
    unread.presets[0].widgets[0].unknown_keys = ['tint'];
    const guarded = presetsForm(unread);
    expect(widgetMovable(guarded.presets[0].widgets[0])).toBe(false);
    expect(moveWidget(guarded.presets[0].widgets, 0, 1)).toBeNull();
    const forced = presetsForm(unread);
    [forced.presets[0].widgets[0], forced.presets[0].widgets[1]] = [forced.presets[0].widgets[1], forced.presets[0].widgets[0]];
    const error = (() => { try { planPresetEdits({ catalog: unread, form: forced }); } catch (thrown) { return thrown; } })();
    expect(error.message).toBe('workshop.presets.refused.reorder');
  });

  it('reorders presets as set edits on the swapped slots and refuses one whose widget tables cannot move', () => {
    // The preset without widgets can be carried; the one with them cannot, because
    // its `[[gm_role_preset.widget]]` tables are their own tables in the document.
    const read = catalog();
    const form = presetsForm(read);
    expect(presetMovable(form.presets[0])).toBe(false);
    expect(presetMovable(form.presets[1])).toBe(true);
    expect(movePreset(form, 1, -1)).toBeNull();
    const plain = catalog();
    plain.presets[0].widgets = [];
    const pair = presetsForm(plain);
    expect(movePreset(pair, 1, -1)).toBe(0);
    expect(pair.presets.map(entry => entry.id)).toEqual(['plain', 'watch']);
    expect(planPresetEdits({ catalog: plain, form: pair })).toEqual([
      { op: 'set', path: ['gm_role_preset', 0, 'id'], value_source: '"plain"' },
      { op: 'set', path: ['gm_role_preset', 0, 'label'], value_source: '"server.gm.plain"' },
      { op: 'remove', path: ['gm_role_preset', 0, 'panels', 1] },
      { op: 'remove', path: ['gm_role_preset', 0, 'panels', 0] },
      { op: 'remove', path: ['gm_role_preset', 0, 'quick_actions', 0] },
      { op: 'remove', path: ['gm_role_preset', 0, 'contacts', 0] },
      { op: 'set', path: ['gm_role_preset', 1, 'id'], value_source: '"watch"' },
      { op: 'set', path: ['gm_role_preset', 1, 'label'], value_source: '"server.gm.watch"' },
      { op: 'put', path: ['gm_role_preset', 1, 'panels'], value_source: '["gm-map-panel", "gm-future-panel"]' },
      { op: 'put', path: ['gm_role_preset', 1, 'quick_actions'], value_source: '["gm-session-pause"]' },
      { op: 'put', path: ['gm_role_preset', 1, 'contacts'], value_source: '["Kestrel"]' },
    ]);
  });

  it('refuses the rules the form can see itself and only when the edit ADDS one', () => {
    const read = catalog();
    expect(refused(read, form => { form.presets[0].id = RESERVED_PRESET_ID; })).toMatchObject({
      message: 'workshop.presets.refused.reserved', detail: 'all' });
    expect(refused(read, form => { form.presets[1].id = 'watch'; })).toMatchObject({
      message: 'workshop.presets.refused.duplicate', detail: 'watch' });
    expect(refused(read, form => { form.presets[0].id = ' '; })).toMatchObject({
      message: 'workshop.presets.refused.empty' });
    expect(refused(read, form => { form.presets[0].label = ''; })).toMatchObject({
      message: 'workshop.presets.refused.label', detail: 'watch' });
    expect(refused(read, form => { form.presets[0].widgets[1].id = 'load'; })).toMatchObject({
      message: 'workshop.presets.refused.widget_duplicate', detail: 'load' });
    expect(refused(read, form => { form.presets[0].widgets[0].id = ''; })).toMatchObject({
      message: 'workshop.presets.refused.widget_empty' });
    expect(refused(read, form => { form.presets[0].widgets[0].label = ''; })).toMatchObject({
      message: 'workshop.presets.refused.widget_label' });
    expect(refused(read, form => { form.presets[0].widgets[0].kind = 'sparkline'; })).toMatchObject({
      message: 'workshop.presets.refused.type', detail: 'sparkline' });
    // A world hand-edited to carry two presets with one id must still let its
    // OTHER presets be edited, one edit at a time.
    const broken = catalog();
    broken.presets[1].id = 'watch';
    expect(plan(broken, form => { form.presets[1].label = 'server.gm.other'; })).toEqual([
      { op: 'set', path: ['gm_role_preset', 1, 'label'], value_source: '"server.gm.other"' },
    ]);
    // …and a THIRD copy of that id is still refused.
    expect(refused(broken, form => { form.presets.push({ ...form.presets[1], index: 7 }); })).toMatchObject({
      message: 'workshop.presets.refused.duplicate', detail: 'watch' });
  });

  it('carries a pre-existing violation across a RENAME, keyed the way the runtime keys it', () => {
    // `presets::compose` keys an empty label by the KEY NAME and an unknown type
    // by the type, so a rename moves neither: the edit introduces nothing and the
    // runtime accepts it. Keying them by the entry's id here instead made the
    // browser refuse an edit the runtime would land — a pre-existing violation
    // read as new, which is the one mistake the introduced-only rule exists to
    // stop, and it left a hand-broken world unrepairable from the form.
    for (const [rule, broken, second] of [
      ['label', read => { read.presets[0].label = ''; }, form => { form.presets[1].label = ''; }],
      ['widget_label', read => { read.presets[0].widgets[0].label = ''; },
        form => { form.presets[0].widgets[1].label = ''; }],
      ['widget_empty', read => { read.presets[0].widgets[0].id = ''; },
        form => { form.presets[0].widgets[1].id = ''; }],
      ['type', read => { read.presets[0].widgets[0].kind = 'sparkline'; },
        form => { form.presets[0].widgets[1].kind = 'sparkline'; }],
      ['widget_duplicate', read => { read.presets[0].widgets[1].id = 'load'; },
        form => { form.presets[0].widgets.push({ ...form.presets[0].widgets[0], index: 9 }); }],
    ]) {
      const read = catalog();
      broken(read);
      expect(plan(read, form => { form.presets[0].id = 'tactical'; }), rule).toEqual([
        { op: 'set', path: ['gm_role_preset', 0, 'id'], value_source: '"tactical"' },
      ]);
      // A SECOND copy of the same violation is still refused, rename and all.
      expect(refused(read, form => { form.presets[0].id = 'tactical'; second(form); }).message, rule)
        .toBe(`workshop.presets.refused.${rule}`);
    }
  });

  it('writes an id, type or label the reading shows as absent with `put` rather than `set`', () => {
    // The catalog gives an ABSENT key the entry's own header line, so the line
    // cannot tell a missing key from an empty one — and the exact-source owner
    // refuses a `set` it cannot locate. A widget with no `label` key is a world
    // the runtime refuses to load and a preset with no `label` key blocks save
    // and export, so repairing one has to be reachable from the form.
    const read = catalog();
    read.presets[1].label = ''; read.presets[1].label_line = 22; // The header line.
    read.presets[0].widgets[0].label = ''; read.presets[0].widgets[0].label_line = 10;
    expect(plan(read, form => {
      form.presets[1].label = 'server.gm.plain';
      form.presets[0].widgets[0].label = 'server.gm.load';
    })).toEqual([
      { op: 'put', path: ['gm_role_preset', 0, 'widget', 0, 'label'], value_source: '"server.gm.load"' },
      { op: 'put', path: ['gm_role_preset', 1, 'label'], value_source: '"server.gm.plain"' },
    ]);
    // A key the reading DOES carry keeps `set`, which holds its decor and type.
    expect(plan(catalog(), form => { form.presets[1].label = 'server.gm.other'; }))
      .toEqual([{ op: 'set', path: ['gm_role_preset', 1, 'label'], value_source: '"server.gm.other"' }]);
  });

  it('removes one of two identical array entries instead of reading the array as a set', () => {
    // `panels`, `quick_actions` and `contacts` are open vocabularies with no
    // duplicate rule in Rust, so a hand-authored repeated entry is reachable and
    // the Remove button beside it must plan an edit. The SURPLUS goes from the
    // tail, so the row the form kept keeps its own line.
    const read = catalog();
    read.presets[0].contacts = [{ index: 0, value: 'Kestrel', line: 7, known: true },
      { index: 1, value: 'Kestrel', line: 7, known: true }];
    expect(plan(read, form => { form.presets[0].contacts.splice(1, 1); }))
      .toEqual([{ op: 'remove', path: ['gm_role_preset', 0, 'contacts', 1] }]);
    expect(plan(read, form => { form.presets[0].contacts = []; })).toEqual([
      { op: 'remove', path: ['gm_role_preset', 0, 'contacts', 1] },
      { op: 'remove', path: ['gm_role_preset', 0, 'contacts', 0] },
    ]);
    // Adding a value the array already holds appends a second occurrence rather
    // than planning nothing at all.
    expect(plan(read, form => { form.presets[0].contacts.push('Kestrel'); })).toEqual([
      { op: 'insert', path: ['gm_role_preset', 0, 'contacts'], index: 2, value_source: '"Kestrel"' },
    ]);
  });

  it('refuses a form whose presets no longer match the reading as stale rather than editing a foreign slot', () => {
    const read = catalog();
    const form = presetsForm(read);
    form.presets.push({ ...form.presets[1], index: 7, id: 'ghost' });
    expect(() => planPresetEdits({ catalog: read, form })).toThrow('workshop.inspector_stale');
  });

  it('raises the browser-owned not-drawn warnings itself and merges them with the runtime\'s findings', () => {
    const read = catalog({ findings: [{ severity: 'error', category: 'preset-unknown-contact',
      message: 'no entity named Kestrel', file: WORLD, line: 7 }] });
    read.presets[0].quick_actions.push({ index: 1, value: 'gm-session-halt', line: 6 });
    read.presets[0].panels.push({ index: 2, value: 'gm-contact-panel', line: 5 });
    read.presets[0].widgets[0].actions = [{ index: 0, value: 'gm-session-detonate', line: 14, known: false }];
    const found = presetFindings(read);
    // The runtime's own finding is kept verbatim; the browser's carry a string id
    // the panel localises, and every one of them is a WARNING.
    expect(found.filter(record => record.severity === 'error')).toHaveLength(1);
    const warnings = found.filter(record => record.severity === 'warning');
    expect(warnings.map(record => [record.category, record.string, record.params])).toEqual([
      ['preset-panel-not-drawn', 'workshop.presets.panel_follows',
        { value: 'gm-contact-panel', leader: 'gm-inspector' }],
      ['preset-panel-not-drawn', 'workshop.presets.panel_not_drawn', { value: 'gm-future-panel' }],
      ['preset-quick-action-not-drawn', 'workshop.presets.quick_action_not_drawn', { value: 'gm-session-halt' }],
      ['preset-quick-action-not-drawn', 'workshop.presets.quick_action_not_drawn', { value: 'gm-session-detonate' }],
    ]);
    // Deterministic, sorted by line and deduped: two readings give one list.
    expect(presetFindings(read)).toEqual(found);
    expect(found.map(record => record.line)).toEqual([5, 5, 6, 7, 14]);
    const twice = catalog();
    twice.findings = [{ severity: 'warning', category: 'x', message: 'once', file: WORLD, line: 2 },
      { severity: 'warning', category: 'x', message: 'once', file: WORLD, line: 2 }];
    expect(presetFindings(twice).filter(record => record.message === 'once')).toHaveLength(1);
    // A panel this build draws raises nothing at all.
    const clean = catalog();
    clean.presets[0].panels = [{ index: 0, value: 'gm-map-panel', line: 5 }];
    expect(presetFindings(clean)).toEqual([]);
    expect(findingsOn(found, WORLD, 7).map(record => record.message)).toEqual(['no entity named Kestrel']);
    expect(findingsOn(found, BASE, 7)).toEqual([]);
  });

  it('offers only the choices the runtime owns and the buttons this build actually draws', () => {
    const read = catalog();
    expect(widgetTypeChoices(read)).toEqual([...GM_WIDGET_TYPES]);
    // A type the browser cannot draw a card for is not offered, and neither is an
    // action id this build has no control for (criterion 4).
    expect(widgetTypeChoices(catalog({ choices: { ...read.choices, widget_types: ['attention', 'sparkline'] } })))
      .toEqual(['attention']);
    expect(widgetActionChoices(read)).toEqual(['gm-session-pause', 'gm-session-resume']);
    expect(widgetActionChoices(catalog({ choices: { ...read.choices,
      widget_actions: ['gm-session-pause', '__hostPause'] } }))).toEqual(['gm-session-pause']);
    expect(contactChoices(read, ['Kestrel'])).toEqual(['Harrow']);
    expect(worldChoices(['scenarios.toml', WORLD, 'assets/entities/mine.toml'], read)).toEqual([
      { path: WORLD, origin: 'draft' }, { path: BASE, origin: 'base' },
    ]);
  });

  it('joins a new preset block with the document\'s own line ending', () => {
    expect(appendPresetBlock('[global]\n', '[[gm_role_preset]]\nid = "watch"\n'))
      .toBe('[global]\n[[gm_role_preset]]\nid = "watch"\n');
    expect(appendPresetBlock('[global]', '[[gm_role_preset]]\n')).toBe('[global]\n[[gm_role_preset]]\n');
    // A CRLF world must not gain a lone LF — the same rule the exact-source owner
    // restores per line.
    expect(appendPresetBlock('[global]\r\ntitle = "a"\r\n', '[[gm_role_preset]]\nid = "watch"\n'))
      .toBe('[global]\r\ntitle = "a"\r\n[[gm_role_preset]]\r\nid = "watch"\r\n');
    expect(appendPresetBlock('', '[[gm_role_preset]]\n')).toBe('[[gm_role_preset]]\n');
  });

  it('maps every runtime refusal to its own category and keeps the runtime\'s words otherwise', () => {
    for (const [rule, category] of [['preset-reserved-id', 'reserved'], ['preset-duplicate-id', 'duplicate'],
      ['preset-empty-id', 'empty'], ['preset-empty-label', 'label'], ['widget-duplicate-id', 'widget_duplicate'],
      ['widget-empty-id', 'widget_empty'], ['widget-empty-label', 'widget_label'], ['widget-unknown-type', 'type'],
      ['widget-key-on-wrong-type', 'key'], ['widget-unknown-band', 'band'], ['widget-unknown-category', 'category'],
      ['widget-unknown-ship', 'ship'], ['widget-unknown-action', 'action'], ['preset-unknown-contact', 'contact']]) {
      expect(refusalStringId(`${rule}: the runtime's own sentence`)).toBe(`workshop.presets.refused.${category}`);
    }
    // The whole-widget rules the runtime also refuses take the catch-all row and
    // its sentence, rather than a rule whose words would name the wrong thing: an
    // `actions` card with no button is not an empty ID, and one button named twice
    // is not a duplicate PRESET. Both are what the word patterns would have said.
    for (const [rule, sentence] of [
      ['widget-empty-actions', "is an 'actions' widget with no actions; an empty button row is a card with nothing on it"],
      ['widget-duplicate-action', "names GM action 'gm-session-pause' twice; one button is one button"],
      ['widget-empty-text', "is a 'note' widget with no text; the text is a strings.csv id"],
      ['widget-invalid-text', "declares text 'Hello there' which is not a strings.csv id"]]) {
      expect(refusalStringId(`${rule}: ${sentence}`)).toBe('workshop.presets.refused.other');
    }
    // A document-level refusal is the shared stale sentence, never a preset rule's
    // words, and so is the exact-source owner's own stale refusal.
    expect(refusalStringId('unknown-document: assets/worlds/gone.toml')).toBe('workshop.inspector_stale');
    expect(refusalStringId('The document changed since it was read')).toBe('workshop.inspector_stale');
    // No prefix: the word patterns, then the catch-all carrying the sentence.
    expect(refusalStringId('the id "all" is reserved')).toBe('workshop.presets.refused.reserved');
    expect(refusalStringId('unknown attention band "puce"')).toBe('workshop.presets.refused.band');
    expect(refusalStringId('the runtime is on fire')).toBe('workshop.presets.refused.other');
    expect(refusalMessage(new Error('boom'))).toBe('boom');
    expect(refusalMessage('boom')).toBe('boom');
  });

  it('has a string-table row for every refusal and finding id it can name', () => {
    // These ids are COMPOSED (`workshop.presets.refused.${rule}`), so neither the
    // t('…') sweep in check-strings nor a panel test that happens not to hit one
    // would notice a missing row — the console would simply render ⟨the.id⟩.
    const composed = ['reserved', 'duplicate', 'empty', 'label', 'widget_duplicate', 'widget_empty', 'widget_label',
      'type', 'key', 'band', 'category', 'ship', 'action', 'contact', 'reorder', 'other']
      .map(rule => `workshop.presets.refused.${rule}`);
    const read = catalog();
    read.presets[0].panels.push({ index: 2, value: 'gm-contact-panel', line: 5 });
    read.presets[0].quick_actions = [{ index: 0, value: 'gm-session-halt', line: 6 }];
    const strings = [...composed, ...presetFindings(read).map(record => record.string).filter(Boolean)];
    expect(strings.length).toBeGreaterThan(16);
    for (const id of strings) expect(t(id), id).not.toContain('⟨');
  });
});
