// @vitest-environment jsdom
import { beforeEach, afterEach, it, expect, vi } from 'vitest';
import { WorkshopDocument } from '../../editor/workshop-document.js';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { mountWorkshopPresets } from '../../gui/workshop-presets-panel.js';
import { GM_ROLE_PRESET_PANEL_IDS, GM_ROLE_PRESET_QUICK_ACTION_IDS,
  GM_WIDGET_ACTION_IDS } from '../../gui/gm-role-presets.js';
import { t } from '../../gui/strings.js';

const MANIFEST = 'scenarios.toml';
const WORLD = 'assets/worlds/mine.toml';
const OTHER = 'assets/worlds/other.toml';
const BASE = 'assets/worlds/base.toml';
const manifestSource = '[pack]\r\nformat = 1\r\nid = "workshop-test"\r\nversion = "1.0.0"\r\nname = "Workshop test"\r\n';
const worldSource = '# keep\n[global]\ntitle = "Mine"\n\n[[gm_role_preset]]\nid = "watch"\nlabel = "server.gm.watch"\n';
const otherSource = '[global]\n';
const FINDING = 'no entity in this world is named Ghost';

const catalog = (overrides = {}) => ({
  path: WORLD, origin: 'draft',
  presets: [
    { index: 0, id: 'watch', id_line: 5, label: 'server.gm.watch', label_line: 6,
      panels: [{ index: 0, value: 'gm-map-panel', line: 7 }, { index: 1, value: 'gm-future-panel', line: 7 }],
      quick_actions: [{ index: 0, value: 'gm-session-pause', line: 8 }],
      contacts: [{ index: 0, value: 'Ghost', line: 9, known: false }],
      widgets: [
        { index: 0, id: 'pressure', id_line: 12, kind: 'attention', kind_line: 13, label: 'server.gm.pressure',
          label_line: 14, band: { value: 'critical', line: 15 }, category: { value: 'damage', line: 16 },
          ship: { value: 'Kestrel', line: 17, known: true }, actions: [], text: null, unknown_keys: [] },
        { index: 1, id: 'load', id_line: 20, kind: 'workload', kind_line: 21, label: 'server.gm.load',
          label_line: 22, band: null, category: null, ship: { value: 'Kestrel', line: 23, known: true },
          actions: [], text: null, unknown_keys: [] },
        { index: 2, id: 'brief', id_line: 26, kind: 'note', kind_line: 27, label: 'server.gm.brief',
          label_line: 28, band: null, category: null, ship: null, actions: [],
          text: { value: 'server.gm.brief.text', line: 29 }, unknown_keys: ['tint'] },
        { index: 3, id: 'buttons', id_line: 32, kind: 'actions', kind_line: 33, label: 'server.gm.buttons',
          label_line: 34, band: null, category: null, ship: null, text: null, unknown_keys: [],
          actions: [{ index: 0, value: 'gm-session-pause', line: 35, known: true },
            { index: 1, value: 'gm-session-detonate', line: 35, known: false }] },
      ],
      unknown_keys: [] },
    { index: 1, id: 'plain', id_line: 38, label: 'server.gm.plain', label_line: 39,
      panels: [], quick_actions: [], contacts: [], widgets: [], unknown_keys: [] },
    { index: 2, id: 'quiet', id_line: 42, label: 'server.gm.quiet', label_line: 43,
      panels: [], quick_actions: [], contacts: [], widgets: [], unknown_keys: [] },
  ],
  choices: { widget_types: ['attention', 'workload', 'actions', 'note'],
    widget_actions: ['gm-session-pause', 'gm-session-resume'],
    bands: ['critical', 'elevated', 'steady'], categories: ['damage', 'comms'], entities: ['Kestrel', 'Harrow'] },
  worlds: [{ path: WORLD, origin: 'draft' }, { path: BASE, origin: 'base' }],
  findings: [{ severity: 'error', category: 'preset-unknown-contact', message: FINDING, file: WORLD, line: 9 }],
  ...overrides });

let draft, runtime, panel, held, changed;
const byId = id => document.getElementById(`workshop-presets-${id}`);
const section = () => document.getElementById('workshop-presets');
const change = node => node.dispatchEvent(new Event('change'));
const type = (node, value) => { node.value = value; node.dispatchEvent(new Event('input')); };
const texts = list => [...list.querySelectorAll('li')].map(row => row.textContent);
const labelOf = control => section().querySelector(`label[for="${control.id}"]`).textContent;
/** A press that reaches the runtime holds the panel until it answers: the hold
 * goes up synchronously with the press and comes down after the answer has been
 * rendered and the controls refreshed. */
const settled = () => vi.waitFor(() => expect(held).toBe(false));
async function read(path = WORLD) {
  byId('world').value = path;
  change(byId('world'));
  await settled();
}
const selectPreset = value => { byId('preset').value = value; change(byId('preset')); };

beforeEach(() => {
  document.body.innerHTML = '<main id="root"></main>';
  draft = new WorkshopDocument(createStoreZip([{ path: MANIFEST, text: manifestSource },
    { path: WORLD, text: worldSource }, { path: OTHER, text: otherSource }]));
  held = false;
  runtime = {
    presets: vi.fn(async () => catalog()),
    editPresets: vi.fn(async (files, request) => `${files[request.document_path]}# edited ${request.edits.length}\n`),
    newPreset: vi.fn(async (id, label) => `[[gm_role_preset]]\nid = "${id}"\nlabel = "${label}"\n`),
  };
  changed = vi.fn();
  panel = mountWorkshopPresets({ root: document.getElementById('root'), runtime, draft: () => draft,
    busy: () => held, setBusy(value) { held = value; panel?.refresh(); }, changed });
});
afterEach(() => panel.dispose());

it('reads one world and shows every preset with its facets and typed widgets', async () => {
  expect(byId('status').textContent).toBe(t('workshop.presets.empty'));
  // The selector is offered before any reading: the draft's own world members.
  expect([...byId('world').options].map(option => option.value)).toEqual([WORLD, OTHER]);
  expect(byId('origin').textContent).toBe(t('workshop.presets.no_reading'));
  await read();
  expect(runtime.presets).toHaveBeenCalledExactlyOnceWith({ [MANIFEST]: manifestSource, [WORLD]: worldSource,
    [OTHER]: otherSource }, WORLD);
  expect(byId('status').textContent).toBe(t('workshop.presets.refreshed'));
  expect(byId('origin').textContent).toContain(t('workshop.presets.world_draft', { path: WORLD }));
  // The runtime's own world list joins the selector, origin and all.
  expect([...byId('world').options].map(option => option.textContent)).toEqual([WORLD, OTHER, `${BASE} (base)`]);
  // Presets: id, label and the controls, with the preset holding widget tables
  // saying its order cannot be changed here rather than holding a dead control.
  expect(byId('preset-0-id').value).toBe('watch');
  expect(byId('preset-0-label').value).toBe('server.gm.watch');
  expect(byId('preset-0-up').disabled).toBe(true);
  expect(byId('preset-0-down').disabled).toBe(true);
  expect(byId('presets').querySelector('[data-preset="0"]').textContent).toContain(t('workshop.presets.immovable'));
  expect(byId('preset-1-up').disabled).toBe(true); // Its neighbour cannot be carried either.
  expect(byId('preset-1-down').disabled).toBe(false);
  expect(byId('preset-2-up').disabled).toBe(false);
  expect(byId('preset-2-down').disabled).toBe(true);
  // A dead Move states its cause on its OWN row: a row that can be carried but
  // whose neighbour cannot says so where a keyboard user reads it, rather than
  // leaving the explanation on the neighbour's row.
  expect(byId('presets').querySelector('[data-preset="1"]').textContent)
    .toContain(t('workshop.presets.immovable_neighbour'));
  expect(byId('presets').querySelector('[data-preset="2"]').textContent)
    .not.toContain(t('workshop.presets.immovable_neighbour'));
  expect(byId('preset-0-remove').getAttribute('aria-label')).toContain('watch');
  // The facets belong to the selected preset, which is the first until chosen.
  expect([...byId('preset').options].map(option => [option.value, option.textContent]))
    .toEqual([['0', 'watch'], ['1', 'plain'], ['2', 'quiet']]);
  expect(byId('preset').value).toBe('0');
  // Panels and quick actions are the BROWSER's vocabulary, in its own order.
  expect(GM_ROLE_PRESET_PANEL_IDS.map((_, index) => labelOf(byId(`panel-${index}`)))).toEqual([...GM_ROLE_PRESET_PANEL_IDS]);
  expect(byId('panel-0').checked).toBe(true);
  expect(byId('panel-1').checked).toBe(false);
  // An authored id this build does not draw is kept as its own row and says so.
  expect(labelOf(byId('panel-extra-0'))).toBe(`gm-future-panel — ${t('workshop.presets.not_drawn')}`);
  expect(byId('panel-extra-0').checked).toBe(true);
  expect(GM_ROLE_PRESET_QUICK_ACTION_IDS.map((_, index) => labelOf(byId(`action-${index}`))))
    .toEqual([...GM_ROLE_PRESET_QUICK_ACTION_IDS]);
  expect(byId('action-0').checked).toBe(true);
  expect(byId('action-1').checked).toBe(false);
  // Contacts: the runtime's finding at its line, beside the row it is about.
  expect(texts(byId('contacts'))[0]).toContain(t('workshop.presets.unknown_contact'));
  expect(texts(byId('contacts'))[0]).toContain(FINDING);
  expect([...byId('add-contact').options].map(option => option.value)).toEqual(['Kestrel', 'Harrow']);
  // Widgets: every control follows the TYPE, so a key is never offered on a type
  // that does not own it.
  expect(byId('widget-0-band').value).toBe('critical');
  expect(byId('widget-0-category').value).toBe('damage');
  expect(byId('widget-0-ship').value).toBe('Kestrel');
  expect(byId('widget-0-text')).toBeNull();
  expect(byId('widget-0-action-0')).toBeNull();
  expect(byId('widget-1-ship').value).toBe('Kestrel');
  expect(byId('widget-1-band')).toBeNull();
  expect(byId('widget-1-category')).toBeNull();
  expect(byId('widget-1-text')).toBeNull();
  expect(byId('widget-2-text').value).toBe('server.gm.brief.text');
  expect(byId('widget-2-ship')).toBeNull();
  expect(byId('widget-3-action-0').checked).toBe(true);
  expect(byId('widget-3-action-1').checked).toBe(false);
  expect(byId('widget-3-band')).toBeNull();
  // A widget carrying a key this form does not read keeps it and cannot be moved.
  expect(byId('widgets').querySelector('[data-kind="note"]').textContent)
    .toContain(t('workshop.presets.unknown_keys', { keys: 'tint' }));
  expect(byId('widget-2-up').disabled).toBe(true);
  expect(byId('widget-2-down').disabled).toBe(true);
  expect(byId('widget-1-down').disabled).toBe(true);
  expect(byId('widget-0-down').disabled).toBe(false);
  expect(byId('widgets').querySelector('[data-kind="workload"]').textContent)
    .toContain(t('workshop.presets.immovable_neighbour_widget'));
  expect(byId('widgets').querySelector('[data-kind="attention"]').textContent)
    .not.toContain(t('workshop.presets.immovable_neighbour_widget'));
  // The type is editable per widget, not only at Add.
  expect(byId('widget-1-type').value).toBe('workload');
  expect([...byId('widget-1-type').options].map(option => option.value))
    .toEqual(['attention', 'workload', 'actions', 'note']);
  // Findings: the runtime's own, plus the two warnings the browser owns, each
  // with its location and its severity as a word.
  const rows = texts(byId('findings'));
  expect(rows.some(row => row.includes(`${WORLD}:9`) && row.includes(t('workshop.severity.error')) && row.includes(FINDING))).toBe(true);
  expect(rows.some(row => row.includes(t('workshop.severity.warning'))
    && row.includes(t('workshop.presets.panel_not_drawn', { value: 'gm-future-panel' })))).toBe(true);
  expect(rows.some(row => row.includes(t('workshop.presets.quick_action_not_drawn', { value: 'gm-session-detonate' })))).toBe(true);
  expect(byId('findings').querySelector('[data-category="preset-panel-not-drawn"]')).not.toBeNull();
});

it('shows a dependency world read-only with its origin and refuses to edit it', async () => {
  runtime.presets.mockResolvedValueOnce(catalog({ path: BASE, origin: 'pack:raiders' }));
  await read();
  expect(byId('origin').textContent).toBe(t('workshop.presets.read_only_origin', { origin: 'pack:raiders' }));
  for (const id of ['preset-0-id', 'preset-0-remove', 'preset-1-down', 'panel-0', 'action-0', 'contact-0-remove',
    'add-contact', 'add-contact-button', 'widget-0-band', 'widget-0-remove', 'add-widget-id', 'add-widget-button',
    'add-preset-id', 'add-preset-button', 'apply']) expect(byId(id).disabled, id).toBe(true);
  byId('apply').click(); byId('add-preset-button').click(); byId('widget-0-remove').click();
  expect(runtime.editPresets).not.toHaveBeenCalled();
  expect(runtime.newPreset).not.toHaveBeenCalled();
  // Reading a dependency world is still allowed.
  expect(byId('refresh').disabled).toBe(false);
});

it('applies a changed label and panel list as ONE history entry that one undo reverts', async () => {
  await read();
  byId('apply').click();
  expect(byId('status').textContent).toBe(t('workshop.presets.unchanged'));
  expect(runtime.editPresets).not.toHaveBeenCalled();
  type(byId('preset-0-label'), 'server.gm.watchful');
  byId('panel-1').checked = true; change(byId('panel-1'));
  expect(draft.canUndo()).toBe(false); // Nothing reaches the draft before Apply.
  byId('apply').click();
  await vi.waitFor(() => expect(changed).toHaveBeenCalledWith(WORLD));
  expect(runtime.editPresets).toHaveBeenCalledExactlyOnceWith(
    { [MANIFEST]: manifestSource, [WORLD]: worldSource, [OTHER]: otherSource },
    { document_path: WORLD, expected_source: worldSource, edits: [
      { op: 'set', path: ['gm_role_preset', 0, 'label'], value_source: '"server.gm.watchful"' },
      { op: 'insert', path: ['gm_role_preset', 0, 'panels'], index: 2, value_source: '"gm-inspector"' },
    ] });
  expect(draft.read(WORLD)).toBe(`${worldSource}# edited 2\n`);
  await settled();
  // The applied source is read again so the forms show what was written.
  expect(runtime.presets).toHaveBeenCalledTimes(2);
  expect(draft.undo()).toBe(WORLD);
  expect(draft.read(WORLD)).toBe(worldSource);
  expect(draft.canUndo()).toBe(false);
});

it('reorders two carriable presets as one edit and keeps keyboard focus in the form', async () => {
  await read();
  byId('preset-1-down').focus();
  byId('preset-1-down').click();
  expect(byId('preset-1-id').value).toBe('quiet');
  expect(byId('preset-2-id').value).toBe('plain');
  // The moved preset is now last, so its own down control is off: focus takes its up control.
  expect(document.activeElement).toBe(byId('preset-2-up'));
  byId('apply').click();
  await vi.waitFor(() => expect(changed).toHaveBeenCalledWith(WORLD));
  expect(runtime.editPresets.mock.calls[0][1].edits).toEqual([
    { op: 'set', path: ['gm_role_preset', 1, 'id'], value_source: '"quiet"' },
    { op: 'set', path: ['gm_role_preset', 1, 'label'], value_source: '"server.gm.quiet"' },
    { op: 'set', path: ['gm_role_preset', 2, 'id'], value_source: '"plain"' },
    { op: 'set', path: ['gm_role_preset', 2, 'label'], value_source: '"server.gm.plain"' },
  ]);
  await settled();
  expect(draft.undo()).toBe(WORLD);
  expect(draft.read(WORLD)).toBe(worldSource);
});

it('removes a preset as one edit and moves focus to the preset that took its place', async () => {
  await read();
  byId('preset-1-remove').focus();
  byId('preset-1-remove').click();
  expect(byId('presets').querySelectorAll('[data-preset]')).toHaveLength(2);
  expect(document.activeElement).toBe(byId('preset-2-id'));
  byId('apply').click();
  await vi.waitFor(() => expect(changed).toHaveBeenCalledWith(WORLD));
  expect(runtime.editPresets.mock.calls[0][1].edits).toEqual([{ op: 'remove', path: ['gm_role_preset', 1] }]);
  await settled();
  expect(draft.undo()).toBe(WORLD);
  expect(draft.read(WORLD)).toBe(worldSource);
});

it('creates a preset from the runtime\'s own block as ONE history entry and refuses the reserved id here', async () => {
  await read();
  type(byId('add-preset-id'), 'all');
  type(byId('add-preset-label'), 'server.gm.all');
  byId('add-preset-button').click();
  expect(byId('status').textContent).toBe(t('workshop.presets.refused.reserved', { detail: 'all' }));
  expect(byId('status').getAttribute('role')).toBe('alert');
  type(byId('add-preset-id'), 'watch');
  byId('add-preset-button').click();
  expect(byId('status').textContent).toBe(t('workshop.presets.refused.duplicate', { detail: 'watch' }));
  type(byId('add-preset-id'), 'relief');
  type(byId('add-preset-label'), '');
  byId('add-preset-button').click();
  expect(byId('status').textContent).toBe(t('workshop.presets.refused.label', { detail: 'relief' }));
  expect(runtime.newPreset).not.toHaveBeenCalled();
  expect(draft.canUndo()).toBe(false);
  type(byId('add-preset-label'), 'server.gm.relief');
  byId('add-preset-button').click();
  await vi.waitFor(() => expect(changed).toHaveBeenCalledWith(WORLD));
  expect(runtime.newPreset).toHaveBeenCalledExactlyOnceWith('relief', 'server.gm.relief');
  expect(draft.read(WORLD)).toBe(`${worldSource}[[gm_role_preset]]\nid = "relief"\nlabel = "server.gm.relief"\n`);
  await settled();
  expect(draft.undo()).toBe(WORLD);
  expect(draft.read(WORLD)).toBe(worldSource);
  expect(draft.canUndo()).toBe(false);
});

it('adds a widget of a chosen type and removes one, each as one edit, and never offers another type\'s key', async () => {
  await read();
  type(byId('add-widget-id'), 'brief');
  byId('add-widget-button').click();
  expect(byId('status').textContent).toBe(t('workshop.presets.refused.widget_duplicate', { detail: 'brief' }));
  expect([...byId('add-widget-type').options].map(option => option.value))
    .toEqual(['attention', 'workload', 'actions', 'note']);
  type(byId('add-widget-id'), 'tempo');
  byId('add-widget-type').value = 'workload';
  byId('add-widget-button').click();
  // The new row carries the ship control its type owns and nothing else.
  expect(byId('widget-4-ship').value).toBe('');
  expect(byId('widget-4-band')).toBeNull();
  expect(byId('widget-4-text')).toBeNull();
  expect(byId('widget-4-action-0')).toBeNull();
  expect(document.activeElement).toBe(byId('widget-4-id'));
  byId('widget-4-ship').value = 'Harrow'; change(byId('widget-4-ship'));
  byId('apply').click();
  await vi.waitFor(() => expect(changed).toHaveBeenCalledWith(WORLD));
  expect(runtime.editPresets.mock.calls[0][1].edits).toEqual([
    { op: 'append_table', path: ['gm_role_preset', 0, 'widget'],
      fields: [['id', '"tempo"'], ['type', '"workload"'], ['label', '"tempo"'], ['ship', '"Harrow"']] },
  ]);
  await settled();
  expect(draft.undo()).toBe(WORLD);
  // Removing one is the mirror press, and the widget that took its place keeps focus.
  await read();
  byId('widget-0-remove').focus();
  byId('widget-0-remove').click();
  expect(document.activeElement).toBe(byId('widget-0-remove'));
  byId('apply').click();
  await vi.waitFor(() => expect(runtime.editPresets).toHaveBeenCalledTimes(2));
  expect(runtime.editPresets.mock.calls[1][1].edits)
    .toEqual([{ op: 'remove', path: ['gm_role_preset', 0, 'widget', 0] }]);
  await settled();
});

it('reorders two carriable widgets as one edit and keeps keyboard focus in the form', async () => {
  await read();
  byId('widget-0-down').focus();
  byId('widget-0-down').click();
  expect([...byId('widgets').querySelectorAll('[data-kind]')].map(group => group.dataset.kind))
    .toEqual(['workload', 'attention', 'note', 'actions']);
  expect([0, 1, 2, 3].map(index => byId(`widget-${index}-id`).value))
    .toEqual(['load', 'pressure', 'brief', 'buttons']);
  // The moved widget's own down control is off (its new neighbour holds an unread
  // key), so focus takes its up control rather than falling to the body.
  expect(document.activeElement).toBe(byId('widget-1-up'));
  byId('apply').click();
  await vi.waitFor(() => expect(changed).toHaveBeenCalledWith(WORLD));
  // A swap is `set`s on the two slots, and the keys the new type does not own go
  // with it rather than becoming a violation this edit introduced.
  expect(runtime.editPresets.mock.calls[0][1].edits).toEqual([
    { op: 'set', path: ['gm_role_preset', 0, 'widget', 0, 'id'], value_source: '"load"' },
    { op: 'set', path: ['gm_role_preset', 0, 'widget', 0, 'type'], value_source: '"workload"' },
    { op: 'set', path: ['gm_role_preset', 0, 'widget', 0, 'label'], value_source: '"server.gm.load"' },
    { op: 'remove', path: ['gm_role_preset', 0, 'widget', 0, 'band'] },
    { op: 'remove', path: ['gm_role_preset', 0, 'widget', 0, 'category'] },
    { op: 'set', path: ['gm_role_preset', 0, 'widget', 1, 'id'], value_source: '"pressure"' },
    { op: 'set', path: ['gm_role_preset', 0, 'widget', 1, 'type'], value_source: '"attention"' },
    { op: 'set', path: ['gm_role_preset', 0, 'widget', 1, 'label'], value_source: '"server.gm.pressure"' },
    { op: 'put', path: ['gm_role_preset', 0, 'widget', 1, 'band'], value_source: '"critical"' },
    { op: 'put', path: ['gm_role_preset', 0, 'widget', 1, 'category'], value_source: '"damage"' },
  ]);
  expect(draft.read(WORLD)).toBe(`${worldSource}# edited 10\n`);
  await settled();
  expect(draft.undo()).toBe(WORLD);
  expect(draft.read(WORLD)).toBe(worldSource);
});

it('changes one widget\'s type, follows it with the controls that type owns, and applies it as one edit', async () => {
  await read();
  expect(byId('widget-1-ship').value).toBe('Kestrel');
  byId('widget-1-type').focus();
  byId('widget-1-type').value = 'note'; change(byId('widget-1-type'));
  // The controls follow the new type: the ship the old one owned is gone and the
  // note text it does own is offered.
  expect(byId('widget-1-ship')).toBeNull();
  expect(byId('widget-1-text').value).toBe('');
  expect(document.activeElement).toBe(byId('widget-1-type'));
  type(byId('widget-1-text'), 'server.gm.load.text');
  byId('apply').click();
  await vi.waitFor(() => expect(changed).toHaveBeenCalledWith(WORLD));
  expect(runtime.editPresets.mock.calls[0][1].edits).toEqual([
    { op: 'set', path: ['gm_role_preset', 0, 'widget', 1, 'type'], value_source: '"note"' },
    { op: 'remove', path: ['gm_role_preset', 0, 'widget', 1, 'ship'] },
    { op: 'put', path: ['gm_role_preset', 0, 'widget', 1, 'text'], value_source: '"server.gm.load.text"' },
  ]);
  await settled();
  expect(draft.undo()).toBe(WORLD);
  expect(draft.read(WORLD)).toBe(worldSource);
});

it('judges a new preset\'s id against the SOURCE and refuses to add one over unapplied edits', async () => {
  await read();
  // A pending Remove lives in form state only, so the id it hides is still in the
  // member the new block is appended to: judging the duplicate by the form would
  // write `id = "watch"` into the draft twice, which no world loads.
  byId('preset-0-remove').click();
  type(byId('add-preset-id'), 'watch');
  type(byId('add-preset-label'), 'server.gm.watch');
  byId('add-preset-button').click();
  expect(byId('status').textContent).toBe(t('workshop.presets.refused.duplicate', { detail: 'watch' }));
  // And an id nobody else has is refused while the removal is unapplied, rather
  // than dropped without a word by the re-read the append forces.
  type(byId('add-preset-id'), 'relief');
  type(byId('add-preset-label'), 'server.gm.relief');
  byId('add-preset-button').click();
  expect(byId('status').textContent).toBe(t('workshop.presets.add_unapplied', { detail: 'relief' }));
  expect(byId('status').getAttribute('role')).toBe('alert');
  expect(runtime.newPreset).not.toHaveBeenCalled();
  expect(draft.canUndo()).toBe(false);
  // Applying the removal clears the way, and the same press then lands.
  byId('apply').click();
  await vi.waitFor(() => expect(changed).toHaveBeenCalledWith(WORLD));
  await settled();
  byId('add-preset-button').click();
  await vi.waitFor(() => expect(runtime.newPreset).toHaveBeenCalledTimes(1));
  expect(runtime.newPreset).toHaveBeenCalledExactlyOnceWith('relief', 'server.gm.relief');
  await settled();
  expect(draft.read(WORLD)).toContain('id = "relief"');
});

it('lets an authored value this build does not draw be removed and applies that as one edit', async () => {
  await read();
  byId('panel-extra-0').focus();
  byId('panel-extra-0').checked = false; change(byId('panel-extra-0'));
  expect(byId('panel-extra-0')).toBeNull();
  expect(document.activeElement).toBe(byId('panel-0'));
  byId('widget-3-action-extra-0').checked = false; change(byId('widget-3-action-extra-0'));
  byId('apply').click();
  await vi.waitFor(() => expect(changed).toHaveBeenCalledWith(WORLD));
  expect(runtime.editPresets.mock.calls[0][1].edits).toEqual([
    { op: 'remove', path: ['gm_role_preset', 0, 'panels', 1] },
    { op: 'remove', path: ['gm_role_preset', 0, 'widget', 3, 'actions', 1] },
  ]);
  await settled();
});

it('adds a contact from the world\'s own entity names and removes one, moving focus as it goes', async () => {
  await read();
  byId('add-contact').value = 'Kestrel';
  byId('add-contact-button').click();
  expect(document.activeElement).toBe(byId('add-contact'));
  // A name the world has carries no unknown-reference note; the authored one the
  // runtime marked unknown keeps its own.
  const contactNames = () => [...byId('contacts').querySelectorAll('li > span')].map(row => row.textContent);
  expect(contactNames()[1]).toBe('Kestrel');
  expect(contactNames()[0]).toContain(t('workshop.presets.unknown_contact'));
  expect([...byId('add-contact').options].map(option => option.value)).toEqual(['Harrow']);
  byId('contact-0-remove').focus();
  byId('contact-0-remove').click();
  expect(document.activeElement).toBe(byId('contact-0-remove'));
  expect(contactNames()).toEqual(['Kestrel']);
  byId('apply').click();
  await vi.waitFor(() => expect(changed).toHaveBeenCalledWith(WORLD));
  expect(runtime.editPresets.mock.calls[0][1].edits).toEqual([
    { op: 'remove', path: ['gm_role_preset', 0, 'contacts', 0] },
    { op: 'insert', path: ['gm_role_preset', 0, 'contacts'], index: 0, value_source: '"Kestrel"' },
  ]);
  await settled();
  expect(draft.undo()).toBe(WORLD);
  expect(draft.read(WORLD)).toBe(worldSource);
});

it('edits the facets of whichever preset the selector names', async () => {
  await read();
  selectPreset('2');
  expect(byId('widgets').querySelectorAll('[data-kind]')).toHaveLength(0);
  expect(byId('panel-0').checked).toBe(false);
  byId('panel-0').checked = true; change(byId('panel-0'));
  byId('apply').click();
  await vi.waitFor(() => expect(changed).toHaveBeenCalledWith(WORLD));
  expect(runtime.editPresets.mock.calls[0][1].edits).toEqual([
    { op: 'put', path: ['gm_role_preset', 2, 'panels'], value_source: '["gm-map-panel"]' },
  ]);
  await settled();
});

it('shows a runtime refusal by category with the draft untouched and refuses a draft that moved during the edit', async () => {
  await read();
  const band = 'widget-unknown-band: unknown attention band "puce" — one of critical, elevated, steady';
  runtime.editPresets.mockRejectedValueOnce(new Error(band));
  byId('widget-0-band').value = 'elevated'; change(byId('widget-0-band'));
  byId('apply').click();
  await settled();
  expect(byId('status').getAttribute('role')).toBe('alert');
  expect(byId('status').textContent).toBe(t('workshop.presets.refused.band', { detail: band }));
  expect(document.activeElement).toBe(byId('status'));
  expect(draft.canUndo()).toBe(false);
  expect(draft.read(WORLD)).toBe(worldSource);
  expect(changed).not.toHaveBeenCalled();
  expect(byId('apply').disabled).toBe(false); // The reading is still current.
  expect(byId('widget-0-band').value).toBe('elevated'); // The form keeps what was asked for.
  // wasm-bindgen may reject with a string.
  runtime.editPresets.mockRejectedValueOnce('preset-reserved-id: the id "all" is reserved');
  byId('apply').click();
  await settled();
  expect(byId('status').textContent).toBe(t('workshop.presets.refused.reserved',
    { detail: 'preset-reserved-id: the id "all" is reserved' }));
  // An unrecognised refusal keeps the runtime's own words, which is all an author has.
  runtime.editPresets.mockRejectedValueOnce(new Error('the runtime is on fire'));
  byId('apply').click();
  await settled();
  expect(byId('status').textContent).toBe(t('workshop.presets.refused.other', { detail: 'the runtime is on fire' }));
  // A draft that moved while the runtime was answering is not written over.
  let complete;
  runtime.editPresets.mockImplementationOnce(() => new Promise(resolve => { complete = resolve; }));
  byId('apply').click();
  await vi.waitFor(() => expect(held).toBe(true));
  draft.edit(OTHER, `${otherSource}# moved\n`);
  complete(`${worldSource}# late\n`);
  await settled();
  expect(draft.read(WORLD)).toBe(worldSource);
  expect(byId('status').textContent).toBe(t('workshop.inspector_stale'));
  expect(changed).not.toHaveBeenCalled();
});

it('disables the forms once any text member changed and hides the panel under a Test hold', async () => {
  await read();
  draft.edit(OTHER, `${otherSource}# newer\n`); panel.refresh();
  expect(byId('status').textContent).toBe(t('workshop.inspector_stale'));
  for (const id of ['preset-0-id', 'preset-0-remove', 'panel-0', 'action-0', 'contact-0-remove', 'widget-0-band',
    'widget-0-remove', 'add-widget-id', 'add-preset-id', 'add-preset-button', 'apply']) {
    expect(byId(id).disabled, id).toBe(true);
  }
  expect(byId('refresh').disabled).toBe(false);
  byId('apply').click(); byId('add-preset-button').click();
  expect(runtime.editPresets).not.toHaveBeenCalled();
  draft.undo(); panel.refresh();
  expect(byId('apply').disabled).toBe(false);
  held = true; panel.refresh({ hidden: true });
  expect(section().hidden).toBe(true);
  for (const id of ['refresh', 'world', 'preset', 'apply', 'preset-0-id']) expect(byId(id).disabled, id).toBe(true);
  byId('apply').click(); byId('refresh').click();
  expect(runtime.editPresets).not.toHaveBeenCalled();
  expect(runtime.presets).toHaveBeenCalledTimes(1);
  held = false; panel.refresh({ hidden: false });
  expect(section().hidden).toBe(false);
  expect(byId('apply').disabled).toBe(false);
});

it('says a draft with no world carries none rather than asking for one to be chosen', async () => {
  draft = new WorkshopDocument(createStoreZip([{ path: MANIFEST, text: manifestSource }]));
  panel.refresh();
  expect(byId('world').options).toHaveLength(0);
  expect(byId('world').disabled).toBe(true);
  expect(byId('refresh').disabled).toBe(true);
  expect(byId('origin').textContent).toBe(t('workshop.presets.no_worlds'));
  byId('refresh').click();
  expect(runtime.presets).not.toHaveBeenCalled();
});

it('does not display a reading that belongs to an older draft', async () => {
  let complete;
  runtime.presets.mockImplementationOnce(() => new Promise(resolve => { complete = resolve; }));
  byId('world').value = WORLD;
  change(byId('world'));
  draft.edit(WORLD, `${worldSource}# changed during read\n`);
  complete(catalog());
  await settled();
  expect(byId('preset-0-id')).toBeNull();
  expect(byId('apply').disabled).toBe(true);
});

/** Criterion 4, the JS half (contract D4): what reaches this panel is
 * presentation. No control, label or planned edit may name a host command route,
 * and the buttons a widget may repeat are the ones this build already draws. */
it('offers no command route and no action id outside the drawn vocabulary', async () => {
  // The runtime names a route among its choices AND the world authors one, which
  // is the case that matters: an authored `__host*` id must be shown so it can be
  // removed, so the claim is about what the form OFFERS, never about whether the
  // page displays the string. (A test forbidding the string would fail on exactly
  // the row that lets an author delete it, while proving nothing about routes.)
  const authored = catalog();
  authored.presets[0].widgets[3].actions = [{ index: 0, value: 'gm-session-pause', line: 35, known: true },
    { index: 1, value: '__hostSessionPause', line: 35, known: false }];
  runtime.presets.mockResolvedValue({ ...authored, choices: { ...authored.choices,
    widget_actions: ['gm-session-pause', '__hostSessionPause', 'gm-mission-abort'] } });
  await read();
  const offered = [...byId('widgets').querySelectorAll('input[type=checkbox]')]
    .filter(box => !box.id.includes('-extra-')).map(box => labelOf(box));
  expect(offered.length).toBeGreaterThan(0);
  for (const id of offered) expect(GM_WIDGET_ACTION_IDS).toContain(id);
  expect(offered).not.toContain('__hostSessionPause');
  // The authored route is a removable row that says this build draws no button
  // for it, and a warning in the same findings list — never a checkbox.
  expect(labelOf(byId('widget-3-action-extra-0'))).toBe(`__hostSessionPause — ${t('workshop.presets.not_drawn')}`);
  expect(byId('widget-3-action-extra-0').checked).toBe(true);
  expect(texts(byId('findings')).some(row => row.includes(
    t('workshop.presets.quick_action_not_drawn', { value: '__hostSessionPause' })))).toBe(true);
  byId('widget-0-ship').value = 'Harrow'; change(byId('widget-0-ship'));
  byId('apply').click();
  await vi.waitFor(() => expect(changed).toHaveBeenCalledWith(WORLD));
  expect(JSON.stringify(runtime.editPresets.mock.calls[0][1].edits)).not.toContain('__host');
  await settled();
});

it('labels every control, gives every button text and every group a legend, and keeps ids unique', async () => {
  await read();
  const controls = [...section().querySelectorAll('input, select')];
  expect(controls.length).toBeGreaterThan(12);
  for (const control of controls) {
    expect(control.id).toBeTruthy();
    expect(section().querySelector(`label[for="${control.id}"]`), control.id).not.toBeNull();
    expect(control.tabIndex).toBeGreaterThanOrEqual(0);
  }
  const buttons = [...section().querySelectorAll('button')];
  expect(buttons.length).toBeGreaterThan(8);
  for (const button of buttons) {
    expect(button.getAttribute('type')).toBe('button');
    expect(button.textContent.trim()).not.toBe('');
  }
  for (const group of section().querySelectorAll('fieldset')) {
    expect(group.firstElementChild.tagName).toBe('LEGEND');
    expect(group.firstElementChild.textContent.trim()).not.toBe('');
  }
  for (const list of section().querySelectorAll('ul')) {
    expect([...list.children].every(child => child.tagName === 'LI')).toBe(true);
  }
  const ids = [...section().querySelectorAll('[id]')].map(element => element.id);
  expect(new Set(ids).size).toBe(ids.length);
  expect(byId('status').getAttribute('tabindex')).toBe('-1');
  // Severity and every state are words in the text, never a colour alone.
  expect(byId('findings').querySelector('li').textContent).toContain(t('workshop.severity.'
    + byId('findings').querySelector('li').dataset.severity));
  expect(byId('presets').querySelector('[data-preset="0"]').textContent).toContain(t('workshop.presets.immovable'));
  expect(section().querySelector('.workshop-presets-undrawn').textContent).toContain(t('workshop.presets.not_drawn'));
});
