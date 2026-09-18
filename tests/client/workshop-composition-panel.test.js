// @vitest-environment jsdom
import { beforeEach, afterEach, it, expect, vi } from 'vitest';
import { WorkshopDocument } from '../../editor/workshop-document.js';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { mountWorkshopComposition } from '../../gui/workshop-composition-panel.js';
import { t } from '../../gui/strings.js';

const MANIFEST = 'scenarios.toml', WORLD = 'assets/worlds/workshop.toml', CHILD = 'assets/worlds/child.toml';
const BASE = 'assets/worlds/base.toml', HULL = 'assets/entities/hull.toml', BASE_CHILD = 'assets/worlds/base_child.toml';
const manifestSource = '# keep\r\n[pack]\r\nformat = 1\r\nid = "workshop-test"\r\nversion = "1.0.0"\r\nname = "Workshop test"\r\n'
  + '[pack.requires]\r\ncontent_id = "phoenix-base"\r\ncontent_epoch = 1\r\n[[scenario]]\r\nid = "workshop-test"\r\n'
  + `world = "${WORLD}"\r\nships = ["${HULL}"]\r\n[[scenario]]\r\nid = "second"\r\nworld = "${WORLD}"\r\nlabel = "world.second.label"\r\n`;
const worldSource = `[global]\ntitle = "Workshop"\nextra_worlds = ["${CHILD}"]\n\n[[available_ships]]\ntemplate_path = "${HULL}"\n`;
const childSource = '[global]\ntitle = "Child"\n';
const FINDING = `${BASE_CHILD} is not in the draft or its dependencies`;
const catalog = () => ({
  manifest: { path: MANIFEST, kind: 'pack', pack: { id: 'workshop-test', name: 'Workshop test', version: '1.0.0', line: 2 }, content: null,
    scenarios: [
      { index: 0, id: 'workshop-test', id_line: 11, world: WORLD, world_line: 12, world_origin: 'draft', label: null,
        ships: [{ path: HULL, line: 13, offered: true }], offered_ships: [HULL] },
      { index: 1, id: 'second', id_line: 15, world: WORLD, world_line: 16, world_origin: 'draft', label: 'world.second.label', ships: [],
        offered_ships: [HULL] },
    ], unknown_keys: ['custom_note'] },
  worlds: [
    { path: WORLD, origin: 'draft', title: 'Workshop', line: 1, extra_worlds: [{ index: 0, path: CHILD, line: 3, origin: 'draft' }],
      script_refs: [{ path: BASE_CHILD, line: 9, kind: 'load', source: 'trigger', origin: null }],
      available_ships: [{ template_path: HULL, line: 6, origin: 'draft' }] },
    { path: CHILD, origin: 'draft', title: 'Child', line: 1, extra_worlds: [], script_refs: [], available_ships: [] },
    { path: BASE, origin: 'base', title: 'Base', line: 1, extra_worlds: [], script_refs: [], available_ships: [] },
  ],
  members: [
    { path: MANIFEST, origin: 'draft', allowed: true, referenced_by: [] },
    { path: WORLD, origin: 'draft', allowed: true, referenced_by: [MANIFEST] },
    { path: CHILD, origin: 'draft', allowed: true, referenced_by: [WORLD] },
    { path: BASE, origin: 'base', allowed: true, referenced_by: [] },
  ],
  choices: { worlds: [{ path: BASE, origin: 'base' }, { path: WORLD, origin: 'draft' }, { path: CHILD, origin: 'draft' }],
    templates: [{ path: HULL, origin: 'draft' }] },
  catalogue: [
    { id: 'workshop-test', world: WORLD, label: null, description: null, ships: [{ template_path: HULL, label: null }], origin: 'draft' },
    { id: 'second', world: WORLD, label: 'world.second.label', description: null, ships: [], origin: 'draft' },
  ],
  findings: [{ severity: 'error', category: 'world-missing-load-reference', message: FINDING, file: WORLD, line: 9 }],
});

let draft, runtime, panel, held, changed, win;
const byId = id => document.getElementById(`workshop-composition-${id}`);
const section = () => document.getElementById('workshop-composition');
const change = node => node.dispatchEvent(new Event('change'));
const type = (node, value) => { node.value = value; node.dispatchEvent(new Event('input')); };
const texts = list => [...list.querySelectorAll('li')].map(row => row.textContent);
/** A press that reaches the runtime holds the panel until it answers: the
 * hold goes up synchronously with the press and comes down after the answer
 * has been rendered and the controls refreshed. */
const settled = () => vi.waitFor(() => expect(held).toBe(false));
async function read() {
  byId('refresh').click();
  await settled();
  expect(byId('root-0-id')).not.toBeNull();
}
beforeEach(() => {
  document.body.innerHTML = '<main id="root"></main>';
  draft = new WorkshopDocument(createStoreZip([{ path: MANIFEST, text: manifestSource }, { path: WORLD, text: worldSource },
    { path: CHILD, text: childSource }]));
  held = false;
  runtime = { composition: vi.fn(async () => catalog()),
    compose: vi.fn(async (files, request) => `${files[request.document_path]}# composed ${request.edits.length}\n`),
    newWorld: vi.fn(async title => `[global]\ntitle = "${title}"\n`) };
  changed = vi.fn();
  win = { confirm: vi.fn(() => true) };
  panel = mountWorkshopComposition({ root: document.getElementById('root'), runtime, draft: () => draft, win,
    busy: () => held, setBusy(value) { held = value; panel?.refresh(); }, changed });
});
afterEach(() => panel.dispose());

it('reads the draft through the runtime and shows roots, worlds with origins, members, the catalogue and findings', async () => {
  expect(byId('status').textContent).toBe(t('workshop.composition.empty'));
  expect(byId('root-0-id')).toBeNull();
  await read();
  expect(runtime.composition).toHaveBeenCalledExactlyOnceWith({ [MANIFEST]: manifestSource, [WORLD]: worldSource, [CHILD]: childSource });
  expect(byId('status').textContent).toBe(t('workshop.composition.refreshed'));
  expect(byId('manifest').textContent).toBe(t('workshop.composition.manifest_pack',
    { path: MANIFEST, id: 'workshop-test', version: '1.0.0', name: 'Workshop test' }));
  expect(byId('root-0-id').value).toBe('workshop-test');
  expect(byId('root-0-world').value).toBe(WORLD);
  // Draft worlds lead the choices; a dependency world carries its origin in the option text.
  expect([...byId('root-0-world').options].map(option => option.textContent)).toEqual([WORLD, CHILD, `${BASE} (base)`]);
  expect(byId('root-0-ship-0').checked).toBe(true);
  expect(byId('root-1-ship-0').checked).toBe(false);
  expect(byId('root-1-label').value).toBe('world.second.label');
  expect(byId('root-0-up').disabled).toBe(true);
  expect(byId('root-0-down').disabled).toBe(false);
  expect(byId('root-1-down').disabled).toBe(true);
  expect(byId('apply-roots').disabled).toBe(false);
  expect(texts(byId('unknown-keys'))).toEqual(['custom_note']);
  expect([...byId('world').options].map(option => option.value)).toEqual([WORLD, CHILD, BASE]);
  expect(byId('world').options[2].textContent).toBe(`${BASE} (base)`);
  expect(byId('world-title').textContent).toBe(t('workshop.composition.world_title', { title: 'Workshop' }));
  expect([...byId('extra-worlds').querySelectorAll('li')].map(row => row.firstChild.textContent)).toEqual([`${CHILD} (draft)`]);
  // Itself and the child it already lists are not offered.
  expect([...byId('add-extra').options].map(option => option.value)).toEqual([BASE]);
  // Script-driven references are listed with their origin and the finding at their line, never edited.
  const refs = texts(byId('script-refs'));
  expect(refs).toHaveLength(1);
  expect(refs[0]).toContain(t('workshop.composition.script_ref', { kind: t('workshop.composition.ref_load'), path: BASE_CHILD,
    line: '9', source: t('workshop.composition.source_trigger'), origin: t('workshop.composition.origin_missing') }));
  expect(refs[0]).toContain(FINDING);
  expect(texts(byId('members'))).toEqual([
    t('workshop.composition.member', { path: MANIFEST, origin: 'draft', allowed: t('workshop.composition.allowed'),
      references: t('workshop.composition.unreferenced') }),
    t('workshop.composition.member', { path: WORLD, origin: 'draft', allowed: t('workshop.composition.allowed'),
      references: t('workshop.composition.referenced_by', { paths: MANIFEST }) }),
    t('workshop.composition.member', { path: CHILD, origin: 'draft', allowed: t('workshop.composition.allowed'),
      references: t('workshop.composition.referenced_by', { paths: WORLD }) }),
    t('workshop.composition.member', { path: BASE, origin: 'base', allowed: t('workshop.composition.allowed'),
      references: t('workshop.composition.unreferenced') }),
  ]);
  expect(texts(byId('catalogue'))).toEqual([
    t('workshop.composition.catalogue_entry', { id: 'workshop-test', label: t('workshop.composition.no_label'), world: WORLD, ships: HULL,
      origin: 'draft' }),
    t('workshop.composition.catalogue_entry', { id: 'second', label: 'world.second.label', world: WORLD,
      ships: t('workshop.composition.no_ships'), origin: 'draft' }),
  ]);
  expect(texts(byId('findings'))).toEqual([`${WORLD}:9 — ${t('workshop.severity.error')}: ${FINDING}`]);
  // A dependency world is shown read-only with its origin.
  byId('world').value = BASE; change(byId('world'));
  expect(section().querySelector('#workshop-composition-world-form .workshop-composition-origin').textContent)
    .toBe(t('workshop.composition.read_only_origin', { origin: 'base' }));
  expect(byId('apply-world').disabled).toBe(true);
  expect(byId('add-extra-button').disabled).toBe(true);
  byId('apply-world').click();
  expect(runtime.compose).not.toHaveBeenCalled();
});

it('applies the roots as ONE compose call on the manifest and ONE history entry that undo reverts', async () => {
  await read();
  // An unchanged form reports the no-op without calling the runtime.
  byId('apply-roots').click();
  expect(byId('status').textContent).toBe(t('workshop.composition.unchanged'));
  expect(runtime.compose).not.toHaveBeenCalled();
  type(byId('root-1-label'), 'world.second.renamed');
  byId('root-0-ship-0').checked = false; change(byId('root-0-ship-0'));
  expect(draft.canUndo()).toBe(false); // Nothing reaches the draft before Apply.
  byId('apply-roots').click();
  await vi.waitFor(() => expect(changed).toHaveBeenCalledWith(MANIFEST));
  expect(runtime.compose).toHaveBeenCalledExactlyOnceWith({ [MANIFEST]: manifestSource, [WORLD]: worldSource, [CHILD]: childSource },
    { document_path: MANIFEST, expected_source: manifestSource, edits: [
      { op: 'remove', path: ['scenario', 0, 'ships', 0] },
      { op: 'set', path: ['scenario', 1, 'label'], value_source: '"world.second.renamed"' },
    ] });
  expect(draft.read(MANIFEST)).toContain('# composed 2');
  // The applied source is read again so the forms show what was written.
  await vi.waitFor(() => expect(runtime.composition).toHaveBeenCalledTimes(2));
  expect(runtime.composition.mock.calls[1][0][MANIFEST]).toContain('# composed 2');
  expect(draft.undo()).toBe(MANIFEST);
  expect(draft.read(MANIFEST)).toBe(manifestSource);
  expect(draft.canUndo()).toBe(false);
});

it('reorders roots as set edits on both slots and keeps keyboard focus on the moved root', async () => {
  await read();
  byId('root-0-down').focus();
  byId('root-0-down').click();
  expect(byId('root-0-id').value).toBe('second');
  expect(byId('root-1-id').value).toBe('workshop-test');
  // The moved root is now last, so its own down control is off: focus takes its up control.
  expect(document.activeElement).toBe(byId('root-1-up'));
  byId('apply-roots').click();
  await vi.waitFor(() => expect(changed).toHaveBeenCalledWith(MANIFEST));
  const { edits } = runtime.compose.mock.calls[0][1];
  expect(edits).toEqual([
    { op: 'set', path: ['scenario', 0, 'id'], value_source: '"second"' },
    { op: 'put', path: ['scenario', 0, 'label'], value_source: '"world.second.label"' },
    { op: 'remove', path: ['scenario', 0, 'ships', 0] },
    { op: 'set', path: ['scenario', 1, 'id'], value_source: '"workshop-test"' },
    { op: 'remove', path: ['scenario', 1, 'label'] },
    { op: 'put', path: ['scenario', 1, 'ships'], value_source: `["${HULL}"]` },
  ]);
  expect(edits.map(edit => edit.op)).not.toContain('append_table');
  expect(edits.filter(edit => edit.op === 'remove' && edit.path.length === 2)).toEqual([]);
});

it('removes a root and adds one as a remove plus an append_table, moving focus after the removal', async () => {
  await read();
  byId('root-0-remove').focus();
  byId('root-0-remove').click();
  expect(byId('root-0-id')).toBeNull();
  expect(document.activeElement).toBe(byId('root-1-id'));
  // A root whose id is already taken is refused before it enters the form.
  type(byId('add-root-id'), 'second');
  byId('add-root-button').click();
  expect(byId('status').getAttribute('role')).toBe('alert');
  expect(byId('status').textContent).toBe(t('workshop.composition.refused.duplicate', { detail: 'second' }));
  expect(byId('root-2-id')).toBeNull();
  type(byId('add-root-id'), 'third');
  byId('add-root-world').value = CHILD; change(byId('add-root-world'));
  byId('add-root-button').click();
  expect(byId('root-2-id').value).toBe('third');
  expect(byId('add-root-id').value).toBe('');
  expect(document.activeElement).toBe(byId('root-2-id'));
  // The new root's world offers no ships, and the box list says so in words.
  expect(texts(byId('root-2-id').closest('fieldset').querySelector('ul'))).toEqual([t('workshop.composition.no_ships')]);
  byId('apply-roots').click();
  await vi.waitFor(() => expect(changed).toHaveBeenCalledWith(MANIFEST));
  expect(runtime.compose.mock.calls[0][1].edits).toEqual([
    { op: 'remove', path: ['scenario', 0] },
    { op: 'append_table', path: ['scenario'], fields: [['id', '"third"'], ['world', `"${CHILD}"`]] },
  ]);
  expect(draft.undo()).toBe(MANIFEST);
});

it('adds an extra world as ONE compose call on the world member and removes one with focus kept in the form', async () => {
  await read();
  byId('add-extra').value = BASE; byId('add-extra-button').click();
  expect(byId('extra-worlds').querySelectorAll('li')).toHaveLength(2);
  expect(byId('extra-1-remove').getAttribute('aria-label')).toContain(BASE);
  expect(byId('add-extra').options).toHaveLength(0);
  expect(document.activeElement).toBe(byId('add-extra'));
  expect(draft.canUndo()).toBe(false);
  byId('apply-world').click();
  await vi.waitFor(() => expect(changed).toHaveBeenCalledWith(WORLD));
  expect(runtime.compose).toHaveBeenCalledExactlyOnceWith({ [MANIFEST]: manifestSource, [WORLD]: worldSource, [CHILD]: childSource },
    { document_path: WORLD, expected_source: worldSource, edits: [{ op: 'insert', path: ['extra_worlds'], index: 1, value_source: `"${BASE}"` }] });
  expect(draft.read(WORLD)).toBe(`${worldSource}# composed 1\n`);
  await vi.waitFor(() => expect(runtime.composition).toHaveBeenCalledTimes(2));
  expect(draft.undo()).toBe(WORLD);
  expect(draft.read(WORLD)).toBe(worldSource);
  // Re-read the reverted draft, then remove the child it lists.
  byId('refresh').click();
  await settled();
  expect(runtime.composition).toHaveBeenCalledTimes(3);
  byId('extra-0-remove').focus();
  byId('extra-0-remove').click();
  expect(texts(byId('extra-worlds'))).toEqual([t('workshop.composition.no_extra_worlds')]);
  expect(document.activeElement).toBe(byId('add-extra'));
  expect([...byId('add-extra').options].map(option => option.value)).toEqual([CHILD, BASE]);
  byId('apply-world').click();
  await settled();
  expect(runtime.compose).toHaveBeenCalledTimes(2);
  expect(runtime.compose.mock.calls[1][1].edits).toEqual([{ op: 'remove', path: ['extra_worlds', 0] }]);
});

it('shows a runtime refusal by category with the draft untouched, and refuses a draft that moved during the edit', async () => {
  await read();
  const cycle = `extra_worlds entry ${BASE} would form a cycle: ${WORLD} -> ${BASE} -> ${WORLD}`;
  runtime.compose.mockRejectedValueOnce(new Error(cycle));
  byId('add-extra').value = BASE; byId('add-extra-button').click();
  byId('apply-world').click();
  await settled();
  expect(byId('status').getAttribute('role')).toBe('alert');
  expect(byId('status').textContent).toBe(t('workshop.composition.refused.cycle', { detail: cycle }));
  expect(document.activeElement).toBe(byId('status'));
  expect(draft.canUndo()).toBe(false);
  expect(draft.read(WORLD)).toBe(worldSource);
  expect(changed).not.toHaveBeenCalled();
  expect(byId('apply-world').disabled).toBe(false); // The reading is still current.
  expect(byId('extra-worlds').querySelectorAll('li')).toHaveLength(2); // The form keeps what was asked for.
  // wasm-bindgen may reject with a string; an unrecognised message keeps the runtime's own words.
  runtime.compose.mockRejectedValueOnce(`world ${BASE} is missing from the candidate and its dependencies`);
  byId('apply-world').click();
  await settled();
  expect(runtime.compose).toHaveBeenCalledTimes(2);
  expect(byId('status').textContent).toBe(t('workshop.composition.refused.missing',
    { detail: `world ${BASE} is missing from the candidate and its dependencies` }));
  runtime.compose.mockRejectedValueOnce(new Error('the runtime is on fire'));
  byId('apply-world').click();
  await settled();
  expect(runtime.compose).toHaveBeenCalledTimes(3);
  expect(byId('status').textContent).toBe(t('workshop.composition.refused.other', { detail: 'the runtime is on fire' }));
  expect(draft.canUndo()).toBe(false);
  // A draft that moved while the runtime was composing is not written over.
  let complete;
  runtime.compose.mockImplementationOnce(() => new Promise(resolve => { complete = resolve; }));
  byId('apply-world').click();
  await vi.waitFor(() => expect(held).toBe(true));
  draft.edit(CHILD, `${childSource}# moved\n`);
  complete(`${worldSource}# late\n`);
  await vi.waitFor(() => expect(held).toBe(false));
  expect(draft.read(WORLD)).toBe(worldSource);
  expect(byId('status').textContent).toBe(t('workshop.inspector_stale'));
  expect(changed).not.toHaveBeenCalled();
});

it('refuses in the form what it can see itself, before the runtime', async () => {
  await read();
  type(byId('root-0-id'), 'second');
  byId('apply-roots').click();
  expect(byId('status').getAttribute('role')).toBe('alert');
  expect(byId('status').textContent).toBe(t('workshop.composition.refused.duplicate', { detail: 'second' }));
  type(byId('root-0-id'), ' ');
  byId('apply-roots').click();
  expect(byId('status').textContent).toBe(t('workshop.composition.refused.empty', { detail: 'id' }));
  expect(runtime.compose).not.toHaveBeenCalled();
  expect(draft.canUndo()).toBe(false);
});

it('disables the forms once any text member changed and hides the panel under a Test hold', async () => {
  await read();
  draft.edit(CHILD, `${childSource}# newer\n`); panel.refresh();
  expect(byId('status').textContent).toBe(t('workshop.inspector_stale'));
  for (const id of ['root-0-id', 'root-0-world', 'root-0-label', 'root-0-ship-0', 'root-0-down', 'root-0-remove', 'add-root-id',
    'add-root-world', 'add-root-button', 'apply-roots', 'world', 'add-extra', 'add-extra-button', 'extra-0-remove', 'apply-world',
    'new-world-title', 'create-world']) expect(byId(id).disabled, id).toBe(true);
  expect(byId('refresh').disabled).toBe(false);
  byId('apply-roots').click(); byId('apply-world').click(); byId('create-world').click();
  expect(runtime.compose).not.toHaveBeenCalled(); expect(runtime.newWorld).not.toHaveBeenCalled();
  draft.undo(); panel.refresh();
  expect(byId('apply-roots').disabled).toBe(false);
  expect(byId('apply-world').disabled).toBe(false);
  held = true; panel.refresh({ hidden: true });
  expect(section().hidden).toBe(true);
  for (const id of ['refresh', 'root-0-id', 'apply-roots', 'apply-world', 'create-world']) expect(byId(id).disabled).toBe(true);
  byId('apply-roots').click(); byId('refresh').click();
  expect(runtime.compose).not.toHaveBeenCalled(); expect(runtime.composition).toHaveBeenCalledTimes(1);
  held = false; panel.refresh({ hidden: false });
  expect(section().hidden).toBe(false);
  expect(byId('apply-roots').disabled).toBe(false);
});

it('does not display a reading that belongs to an older draft', async () => {
  let complete;
  runtime.composition.mockImplementationOnce(() => new Promise(resolve => { complete = resolve; }));
  byId('refresh').click();
  draft.edit(WORLD, `${worldSource}# changed during read\n`);
  complete(catalog());
  await vi.waitFor(() => expect(held).toBe(false));
  expect(byId('root-0-id')).toBeNull();
  expect(byId('apply-roots').disabled).toBe(true);
});

it('creates a world at its slug path from the runtime skeleton and refuses an existing path', async () => {
  await read();
  type(byId('new-world-title'), 'Workshop');
  byId('create-world').click();
  expect(byId('status').getAttribute('role')).toBe('alert');
  expect(byId('status').textContent).toBe(t('workshop.composition.world_exists', { detail: WORLD }));
  expect(runtime.newWorld).not.toHaveBeenCalled();
  type(byId('new-world-title'), '!!!');
  byId('create-world').click();
  expect(byId('status').textContent).toBe(t('workshop.composition.invalid_title'));
  type(byId('new-world-title'), 'Harrow Reach');
  byId('create-world').click();
  const path = 'assets/worlds/harrow-reach.toml';
  await vi.waitFor(() => expect(changed).toHaveBeenCalledWith(path));
  expect(runtime.newWorld).toHaveBeenCalledExactlyOnceWith('Harrow Reach');
  expect(draft.read(path)).toBe('[global]\ntitle = "Harrow Reach"\n');
  expect(byId('new-world-title').value).toBe('');
  await settled();
  expect(runtime.composition).toHaveBeenCalledTimes(2);
  expect(runtime.composition.mock.calls[1][0][path]).toBe('[global]\ntitle = "Harrow Reach"\n');
  expect(draft.undo()).toBe(path);
  expect(draft.paths()).not.toContain(path);
});

it('keeps the unapplied edits of one form while the other form is applied', async () => {
  await read();
  type(byId('root-1-label'), 'world.second.renamed');
  byId('add-extra').value = BASE; byId('add-extra-button').click();
  byId('apply-world').click();
  await settled();
  expect(changed).toHaveBeenCalledWith(WORLD);
  expect(runtime.composition).toHaveBeenCalledTimes(2);
  // The world form shows what was written; the manifest did not move, so its form keeps the typing.
  expect([...byId('extra-worlds').querySelectorAll('li')].map(row => row.firstChild.textContent)).toEqual([`${CHILD} (draft)`]);
  expect(byId('root-1-label').value).toBe('world.second.renamed');
  byId('add-extra').value = BASE; byId('add-extra-button').click();
  byId('apply-roots').click();
  await settled();
  expect(changed).toHaveBeenCalledWith(MANIFEST);
  expect(runtime.composition).toHaveBeenCalledTimes(3);
  expect(runtime.compose.mock.calls[1][1].edits).toEqual([{ op: 'set', path: ['scenario', 1, 'label'], value_source: '"world.second.renamed"' }]);
  expect(byId('root-1-label').value).toBe('world.second.label');
  expect(byId('extra-worlds').querySelectorAll('li')).toHaveLength(2);
  // A member that changed under the form is read afresh, typing and all.
  draft.edit(WORLD, `${worldSource}# elsewhere\n`);
  byId('refresh').click();
  await settled();
  expect(runtime.composition).toHaveBeenCalledTimes(4);
  expect(byId('extra-worlds').querySelectorAll('li')).toHaveLength(1);
});

it('follows a root world change with that world\'s offered ships and keeps a ship the world does not offer as a clearable row', async () => {
  await read();
  byId('root-0-world').value = CHILD; change(byId('root-0-world'));
  expect(document.activeElement).toBe(byId('root-0-world'));
  // The reading's ship is not offered by the child world: it stays checked, named, and clearable.
  expect(byId('root-0-ship-0')).toBeNull();
  const unoffered = byId('root-0-unoffered-0');
  expect(unoffered.checked).toBe(true);
  expect(section().querySelector(`label[for="${unoffered.id}"]`).textContent).toContain(t('workshop.composition.ship_not_offered'));
  unoffered.focus();
  unoffered.checked = false; change(unoffered);
  expect(byId('root-0-unoffered-0')).toBeNull();
  expect(document.activeElement).toBe(byId('root-0-label'));
  byId('apply-roots').click();
  await vi.waitFor(() => expect(changed).toHaveBeenCalledWith(MANIFEST));
  expect(runtime.compose.mock.calls[0][1].edits).toEqual([
    { op: 'set', path: ['scenario', 0, 'world'], value_source: `"${CHILD}"` },
    { op: 'remove', path: ['scenario', 0, 'ships', 0] },
  ]);
});

it('labels every control, gives every button text and every group a legend, and keeps ids unique', async () => {
  await read();
  byId('root-0-world').value = CHILD; change(byId('root-0-world'));
  const controls = [...section().querySelectorAll('input, select')];
  expect(controls.length).toBeGreaterThan(10);
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
  for (const list of section().querySelectorAll('ul')) expect([...list.children].every(child => child.tagName === 'LI')).toBe(true);
  const ids = [...section().querySelectorAll('[id]')].map(element => element.id);
  expect(new Set(ids).size).toBe(ids.length);
  expect(byId('status').getAttribute('tabindex')).toBe('-1');
  // Origins and severities are words in the text, never a colour alone.
  expect(byId('findings').querySelector('li').textContent).toContain(t('workshop.severity.error'));
  expect(byId('members').querySelector('[data-origin="base"]').textContent).toContain('base');
});
