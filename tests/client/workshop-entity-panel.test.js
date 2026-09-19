// @vitest-environment jsdom
import { beforeEach, afterEach, it, expect, vi } from 'vitest';
import { WorkshopDocument } from '../../editor/workshop-document.js';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { mountWorkshopEntity } from '../../gui/workshop-entity-panel.js';
import { t } from '../../gui/strings.js';

const MANIFEST = 'scenarios.toml';
const TEMPLATE = 'assets/entities/mine.toml';
const FRAGMENT = 'assets/entities/fragments/ai/base.toml';
const OTHER = 'assets/entities/fragments/ai/captain.toml';
const BASE_HULL = 'assets/entities/alliance_cruiser.toml';
const manifestSource = '[pack]\r\nformat = 1\r\nid = "workshop-test"\r\nversion = "1.0.0"\r\nname = "Workshop test"\r\n';
const templateSource = '# keep\nname = "mine"\nincludes = ["fragments/ai/base.toml"]\n\n[hull]\nhull_integrity = 100\n';
const fragmentSource = '[ai_profile]\nstance = "escort"\n';
const otherSource = '[comms]\nchannel = "fleet"\n';
const FINDING = `${OTHER} is not in the draft or its dependencies`;

const composition = (overrides = {}) => ({
  path: TEMPLATE, origin: 'draft', resolvable: true, error: null,
  includes: [{ index: 0, authored: 'fragments/ai/base.toml', canonical: FRAGMENT, line: 3, origin: 'draft' }],
  sources: [FRAGMENT, TEMPLATE],
  components: [
    // The runtime's own shape: the availability flag, and the default's text
    // beside it exactly when the flag is set (entity.rs `ComponentView`).
    { key: 'hull', local: true, local_line: 5, inherited_from: null, skeleton: true,
      skeleton_source: '{ hull_integrity = 100 }' },
    { key: 'ai_profile', local: false, local_line: null, inherited_from: FRAGMENT, skeleton: true,
      skeleton_source: '{ stance = "escort" }' },
    { key: 'comms', local: false, local_line: null, inherited_from: null, skeleton: true,
      skeleton_source: '{ channel = "open" }' },
    { key: 'shape', local: false, local_line: null, inherited_from: null, skeleton: false, skeleton_source: null },
  ],
  fields: [
    { address: 'hull.hull_integrity', source: TEMPLATE, chain: [TEMPLATE], local: true, value_source: '100', line: 6,
      kind: 'integer', materialisable: false },
    { address: 'ai_profile.stance', source: FRAGMENT, chain: [FRAGMENT, TEMPLATE], local: false,
      value_source: '"escort"', line: null, kind: 'string', materialisable: true },
    { address: 'system[id=helm-thrust].ai_only', source: TEMPLATE, chain: [TEMPLATE], local: true, value_source: 'true',
      line: 9, kind: 'boolean', materialisable: false },
    // A local LIST: the exact-source `set` route is scalars only, so this row is
    // shown and not typed into either.
    { address: 'tags', source: TEMPLATE, chain: [TEMPLATE], local: true, value_source: '["ship"]', line: 2,
      kind: 'array', materialisable: false },
    // An INHERITED value inside a keyed array entry this template does not
    // author: the runtime refuses materialising it, and says so in the catalog.
    { address: 'station[id=bridge].name', source: FRAGMENT, chain: [FRAGMENT, TEMPLATE], local: false,
      value_source: '"Bridge"', line: null, kind: 'string', materialisable: false },
  ],
  supported_components: ['hull', 'ai_profile', 'comms', 'shape'],
  fragment_choices: [{ path: FRAGMENT, origin: 'draft' }, { path: OTHER, origin: 'draft' },
    { path: BASE_HULL, origin: 'base' }],
  findings: [{ severity: 'error', category: 'include-missing', message: FINDING, file: TEMPLATE, line: 3 }],
  ...overrides,
});
const twoIncludes = () => composition({ includes: [
  { index: 0, authored: 'fragments/ai/base.toml', canonical: FRAGMENT, line: 3, origin: 'draft' },
  { index: 1, authored: 'fragments/ai/captain.toml', canonical: OTHER, line: 4, origin: 'draft' },
] });

let draft, runtime, panel, held, changed, win, preview, test;
const byId = id => document.getElementById(`workshop-entity-${id}`);
const section = () => document.getElementById('workshop-entity');
const change = node => node.dispatchEvent(new Event('change'));
const type = (node, value) => { node.value = value; node.dispatchEvent(new Event('input')); };
const texts = list => [...list.querySelectorAll('li')].map(row => row.textContent);
const labelOf = control => section().querySelector(`label[for="${control.id}"]`).textContent;
/** A press that reaches the runtime holds the panel until it answers: the hold
 * goes up synchronously with the press and comes down after the answer has been
 * rendered and the controls refreshed. */
const settled = () => vi.waitFor(() => expect(held).toBe(false));
async function read(path = TEMPLATE) {
  byId('template').value = path;
  change(byId('template'));
  await settled();
}

beforeEach(() => {
  document.body.innerHTML = '<main id="root"></main>';
  draft = new WorkshopDocument(createStoreZip([{ path: MANIFEST, text: manifestSource },
    { path: TEMPLATE, text: templateSource }, { path: FRAGMENT, text: fragmentSource }, { path: OTHER, text: otherSource }]));
  held = false;
  runtime = {
    entity: vi.fn(async () => composition()),
    editEntity: vi.fn(async (files, request) => `${files[request.document_path]}# edited ${request.edits.length}\n`),
    materialiseEntity: vi.fn(async (files, path, address) => `${files[path]}# materialised ${address}\n`),
  };
  changed = vi.fn();
  win = { confirm: vi.fn(() => true) };
  preview = vi.fn(() => true);
  test = vi.fn(() => true);
  panel = mountWorkshopEntity({ root: document.getElementById('root'), runtime, draft: () => draft, win,
    busy: () => held, setBusy(value) { held = value; panel?.refresh(); }, changed, preview, test });
});
afterEach(() => panel.dispose());

it('reads one template and shows its includes, components and every field with the member that owns it', async () => {
  expect(byId('status').textContent).toBe(t('workshop.entity.empty'));
  // The selector is offered before any reading: the draft's own templates, sorted.
  expect([...byId('template').options].map(option => option.value)).toEqual([FRAGMENT, OTHER, TEMPLATE]);
  expect(byId('origin').textContent).toBe(t('workshop.entity.no_reading'));
  await read();
  expect(byId('refresh').disabled).toBe(false);
  expect(runtime.entity).toHaveBeenCalledExactlyOnceWith({ [MANIFEST]: manifestSource, [TEMPLATE]: templateSource,
    [FRAGMENT]: fragmentSource, [OTHER]: otherSource }, TEMPLATE);
  expect(byId('status').textContent).toBe(t('workshop.entity.refreshed'));
  // The runtime's own fragment list joins the selector, origin and all.
  expect([...byId('template').options].map(option => option.textContent))
    .toEqual([FRAGMENT, OTHER, TEMPLATE, `${BASE_HULL} (base)`]);
  expect(byId('origin').textContent).toContain(t('workshop.entity.template_draft', { path: TEMPLATE }));
  expect(byId('origin').textContent).toContain(t('workshop.entity.sources', { paths: `${FRAGMENT}, ${TEMPLATE}` }));
  // Includes: the text the document holds, the member it resolves to, its origin
  // and the finding at its line.
  const include = texts(byId('includes'))[0];
  expect(include).toContain(t('workshop.entity.include', { authored: 'fragments/ai/base.toml', path: FRAGMENT, origin: 'draft' }));
  expect(include).toContain(FINDING);
  expect(byId('include-0-up').disabled).toBe(true);
  expect(byId('include-0-down').disabled).toBe(true);
  expect(byId('include-0-remove').disabled).toBe(false);
  // Itself and the fragment it already includes are not offered.
  expect([...byId('add-include').options].map(option => option.value)).toEqual([OTHER, BASE_HULL]);
  // Components: local, inherited and absent said in words, with Add only where
  // the runtime has a default and Remove only where there is local text.
  expect([...byId('components').querySelectorAll('li > span')].map(row => row.textContent)).toEqual([
    t('workshop.entity.component_local', { key: 'hull', line: '5' }),
    `${t('workshop.entity.component_inherited', { key: 'ai_profile', source: FRAGMENT })} ${t('workshop.entity.component_materialise_first')}`,
    t('workshop.entity.component_absent', { key: 'comms' }),
    `${t('workshop.entity.component_absent', { key: 'shape' })} ${t('workshop.entity.no_skeleton')}`,
  ]);
  expect(byId('component-hull-remove').disabled).toBe(false);
  expect(byId('component-hull-add')).toBeNull();
  expect(byId('component-ai_profile-add')).toBeNull();
  expect(byId('component-ai_profile-remove')).toBeNull();
  expect(byId('component-comms-add').disabled).toBe(false);
  expect(byId('component-shape-add')).toBeNull();
  // Fields: grouped by their component, every row naming its owner and chain.
  expect([...byId('fields').querySelectorAll('legend')].map(legend => legend.textContent))
    .toEqual(['hull', 'ai_profile', 'system', 'tags', 'station'].map(key => t('workshop.entity.field_group', { key })));
  expect(byId('field-0-value').value).toBe('100');
  expect(byId('field-0-value').disabled).toBe(false);
  expect(labelOf(byId('field-0-value'))).toContain(t('workshop.entity.field_local', { line: '6' }));
  expect(byId('field-0-materialise')).toBeNull();
  // Inherited is read-only with its owner and chain until it is materialised.
  expect(byId('field-1-value').value).toBe('"escort"');
  expect(byId('field-1-value').disabled).toBe(true);
  expect(labelOf(byId('field-1-value'))).toContain(t('workshop.entity.field_inherited', { source: FRAGMENT }));
  expect(labelOf(byId('field-1-value'))).toContain(t('workshop.entity.field_chain', { chain: `${FRAGMENT} → ${TEMPLATE}` }));
  expect(labelOf(byId('field-1-value'))).toContain(t('workshop.entity.field_kind', { kind: 'string' }));
  expect(byId('field-1-materialise').disabled).toBe(false);
  // A keyed address is authored by key, so it is shown rather than typed into.
  expect(byId('field-2-value').disabled).toBe(true);
  expect(labelOf(byId('field-2-value'))).toContain(t('workshop.entity.field_keyed'));
  expect(byId('field-2-materialise')).toBeNull();
  // …and so is a list, for its own reason, which the row names rather than
  // borrowing the keyed sentence.
  expect(byId('field-3-value').disabled).toBe(true);
  expect(labelOf(byId('field-3-value'))).toContain(t('workshop.entity.field_structured', { kind: 'array' }));
  expect(labelOf(byId('field-3-value'))).not.toContain(t('workshop.entity.field_keyed'));
  expect(byId('field-3-materialise')).toBeNull();
  // An inherited value the runtime cannot write into this template yet offers no
  // Materialise at all — a control whose every press is refused is worse than
  // none — and the row says which of the two reasons it is.
  expect(byId('field-4-value').disabled).toBe(true);
  expect(byId('field-4-materialise')).toBeNull();
  expect(labelOf(byId('field-4-value'))).toContain(t('workshop.entity.field_no_entry'));
  expect(labelOf(byId('field-4-value'))).toContain(t('workshop.entity.field_inherited', { source: FRAGMENT }));
  expect(texts(byId('findings'))).toEqual([`${TEMPLATE}:3 — ${t('workshop.severity.error')}: ${FINDING}`]);
});

it('shows a dependency template read-only with its origin and refuses to edit it', async () => {
  runtime.entity.mockResolvedValueOnce(composition({ path: BASE_HULL, origin: 'base', resolvable: false,
    error: 'include-missing: fragments/ai/captain.toml' }));
  await read();
  expect(byId('origin').textContent).toContain(t('workshop.entity.read_only_origin', { origin: 'base' }));
  expect(byId('origin').textContent).toContain(t('workshop.entity.unresolvable',
    { detail: 'include-missing: fragments/ai/captain.toml' }));
  for (const id of ['include-0-remove', 'add-include', 'add-include-button', 'apply-includes', 'component-hull-remove',
    'component-comms-add', 'field-0-value', 'field-1-materialise', 'apply-fields']) expect(byId(id).disabled, id).toBe(true);
  byId('apply-includes').click(); byId('component-comms-add').click(); byId('field-1-materialise').click();
  expect(runtime.editEntity).not.toHaveBeenCalled();
  expect(runtime.materialiseEntity).not.toHaveBeenCalled();
  // Reading a dependency template is still allowed, and so is previewing it.
  expect(byId('refresh').disabled).toBe(false);
  expect(byId('preview').disabled).toBe(false);
});

it('materialises an inherited field as ONE history entry that one undo reverts', async () => {
  await read();
  expect(draft.canUndo()).toBe(false);
  byId('field-1-materialise').click();
  await vi.waitFor(() => expect(changed).toHaveBeenCalledWith(TEMPLATE));
  expect(runtime.materialiseEntity).toHaveBeenCalledExactlyOnceWith({ [MANIFEST]: manifestSource,
    [TEMPLATE]: templateSource, [FRAGMENT]: fragmentSource, [OTHER]: otherSource }, TEMPLATE, 'ai_profile.stance');
  expect(draft.read(TEMPLATE)).toBe(`${templateSource}# materialised ai_profile.stance\n`);
  // Only the local member is written: the fragment it inherited from is untouched.
  expect(draft.read(FRAGMENT)).toBe(fragmentSource);
  await settled();
  // The applied source is read again so the form shows what was written.
  expect(runtime.entity).toHaveBeenCalledTimes(2);
  expect(runtime.entity.mock.calls[1][0][TEMPLATE]).toContain('# materialised ai_profile.stance');
  expect(draft.undo()).toBe(TEMPLATE);
  expect(draft.read(TEMPLATE)).toBe(templateSource);
  expect(draft.canUndo()).toBe(false);
});

it('adds, reorders and removes includes as one edit call each and keeps keyboard focus in the form', async () => {
  runtime.entity.mockResolvedValue(twoIncludes());
  await read();
  // An unchanged form reports the no-op without calling the runtime.
  byId('apply-includes').click();
  expect(byId('status').textContent).toBe(t('workshop.entity.unchanged'));
  expect(runtime.editEntity).not.toHaveBeenCalled();
  byId('include-0-down').focus();
  byId('include-0-down').click();
  expect(texts(byId('includes'))[0]).toContain('fragments/ai/captain.toml');
  // The moved entry is now last, so its own down control is off: focus takes its up control.
  expect(document.activeElement).toBe(byId('include-1-up'));
  expect(draft.canUndo()).toBe(false); // Nothing reaches the draft before Apply.
  byId('apply-includes').click();
  await vi.waitFor(() => expect(changed).toHaveBeenCalledWith(TEMPLATE));
  expect(runtime.editEntity).toHaveBeenCalledExactlyOnceWith({ [MANIFEST]: manifestSource, [TEMPLATE]: templateSource,
    [FRAGMENT]: fragmentSource, [OTHER]: otherSource },
  { document_path: TEMPLATE, expected_source: templateSource, edits: [
    { op: 'set', path: ['includes', 0], value_source: '"fragments/ai/captain.toml"' },
    { op: 'set', path: ['includes', 1], value_source: '"fragments/ai/base.toml"' },
  ] });
  expect(draft.read(TEMPLATE)).toBe(`${templateSource}# edited 2\n`);
  expect(draft.undo()).toBe(TEMPLATE);
  expect(draft.read(TEMPLATE)).toBe(templateSource);
  // Add: the chosen fragment enters the form as the relative text the resolver reads.
  await read();
  byId('add-include').value = BASE_HULL;
  byId('add-include-button').click();
  expect(byId('includes').querySelectorAll('li')).toHaveLength(3);
  expect(byId('include-2-remove').getAttribute('aria-label')).toContain('alliance_cruiser.toml');
  expect(document.activeElement).toBe(byId('add-include'));
  byId('apply-includes').click();
  await settled();
  expect(runtime.editEntity.mock.calls[1][1].edits).toEqual([
    { op: 'insert', path: ['includes'], index: 2, value_source: '"alliance_cruiser.toml"' },
  ]);
  // Remove: the entry that took its place keeps the focus.
  await read();
  byId('include-0-remove').focus();
  byId('include-0-remove').click();
  expect(byId('includes').querySelectorAll('li')).toHaveLength(1);
  expect(document.activeElement).toBe(byId('include-0-remove'));
  byId('apply-includes').click();
  await settled();
  expect(runtime.editEntity.mock.calls[2][1].edits).toEqual([
    { op: 'set', path: ['includes', 0], value_source: '"fragments/ai/captain.toml"' },
    { op: 'remove', path: ['includes', 1] },
  ]);
});

it('adds and removes a component as one press each, moving focus to the control that replaces it', async () => {
  await read();
  const local = composition({ components: composition().components.map(entry => (entry.key === 'comms'
    ? { ...entry, local: true, local_line: 9 } : entry)) });
  runtime.entity.mockResolvedValueOnce(local);
  byId('component-comms-add').focus();
  byId('component-comms-add').click();
  await vi.waitFor(() => expect(changed).toHaveBeenCalledWith(TEMPLATE));
  expect(runtime.editEntity).toHaveBeenCalledExactlyOnceWith(expect.anything(),
    { document_path: TEMPLATE, expected_source: templateSource,
      edits: [{ op: 'put', path: ['comms'], value_source: '{ channel = "open" }' }] });
  await settled();
  expect(byId('component-comms-add')).toBeNull();
  expect(document.activeElement).toBe(byId('component-comms-remove'));
  expect(draft.undo()).toBe(TEMPLATE);
  // Removing the local table is the mirror press, and the Add that replaces it takes focus.
  await read();
  runtime.entity.mockResolvedValueOnce(composition({ components: composition().components.map(entry => (entry.key === 'hull'
    ? { ...entry, local: false, local_line: null } : entry)) }));
  byId('component-hull-remove').click();
  await vi.waitFor(() => expect(runtime.editEntity).toHaveBeenCalledTimes(2));
  expect(runtime.editEntity.mock.calls[1][1].edits).toEqual([{ op: 'remove', path: ['hull'] }]);
  await settled();
  expect(document.activeElement).toBe(byId('component-hull-add'));
});

it('says when a local component is ALSO authored by a fragment and still lets the override be dropped', async () => {
  // The central case of composition: the template authors part of a component a
  // fragment authors too. The row used to read as merely local, and its Remove
  // was refused by the runtime on every press.
  const override = composition({ components: composition().components.map(entry => (entry.key === 'comms'
    ? { ...entry, local: true, local_line: 9, inherited_from: FRAGMENT } : entry)) });
  runtime.entity.mockResolvedValue(override);
  await read();
  const row = byId('components').querySelector('[data-component="comms"]');
  expect(row.dataset.state).toBe('override');
  expect(row.querySelector('span').textContent).toBe(
    `${t('workshop.entity.component_local', { key: 'comms', line: '9' })} `
    + t('workshop.entity.component_override', { source: FRAGMENT }));
  // No Add — it is authored here — and a Remove that names what dropping it does.
  expect(byId('component-comms-add')).toBeNull();
  expect(byId('component-comms-remove').disabled).toBe(false);
  expect(byId('component-comms-remove').getAttribute('aria-label')).toContain(FRAGMENT);
  byId('component-comms-remove').click();
  await vi.waitFor(() => expect(changed).toHaveBeenCalledWith(TEMPLATE));
  expect(runtime.editEntity).toHaveBeenCalledExactlyOnceWith(expect.anything(),
    { document_path: TEMPLATE, expected_source: templateSource, edits: [{ op: 'remove', path: ['comms'] }] });
  await settled();
});

it('keeps focus inside the panel after removing a component the runtime has no default for', async () => {
  // No skeleton means no Add takes the Remove's place, and every stable control
  // is disabled while the hold is up: focus fell to the body and a keyboard user
  // Tabbed from the top of the page again.
  runtime.entity.mockResolvedValueOnce(composition({ components: composition().components.map(entry => (
    entry.key === 'shape' ? { ...entry, local: true, local_line: 11 } : entry)) }));
  await read();
  byId('component-shape-remove').focus();
  byId('component-shape-remove').click();
  await vi.waitFor(() => expect(changed).toHaveBeenCalledWith(TEMPLATE));
  await settled();
  expect(byId('component-shape-remove')).toBeNull();
  expect(byId('component-shape-add')).toBeNull();
  // The named stable fallback, which is only reachable because focus moves after
  // the hold comes down and the controls are enabled again.
  expect(document.activeElement).toBe(byId('refresh'));
  expect(document.activeElement.tagName).not.toBe('BODY');
});

it('applies a changed local scalar as ONE set edit and never writes an inherited value', async () => {
  await read();
  byId('apply-fields').click();
  expect(byId('status').textContent).toBe(t('workshop.entity.unchanged'));
  expect(runtime.editEntity).not.toHaveBeenCalled();
  type(byId('field-0-value'), '140');
  byId('apply-fields').click();
  await vi.waitFor(() => expect(changed).toHaveBeenCalledWith(TEMPLATE));
  expect(runtime.editEntity).toHaveBeenCalledExactlyOnceWith(expect.anything(),
    { document_path: TEMPLATE, expected_source: templateSource,
      edits: [{ op: 'set', path: ['hull', 'hull_integrity'], value_source: '140' }] });
  expect(draft.undo()).toBe(TEMPLATE);
  // The inherited row cannot be typed into at all, so no edit can name it.
  await read();
  byId('field-1-value').value = '"picket"';
  byId('field-1-value').dispatchEvent(new Event('input'));
  byId('apply-fields').click();
  expect(byId('status').textContent).toBe(t('workshop.entity.refused.inherited', { detail: FRAGMENT }));
  expect(byId('status').getAttribute('role')).toBe('alert');
  expect(runtime.editEntity).toHaveBeenCalledTimes(1);
  expect(draft.read(TEMPLATE)).toBe(templateSource);
});

it('shows a runtime refusal by category with the draft untouched, and refuses a draft that moved during the edit', async () => {
  await read();
  const cycle = `include-cycle: fragments/ai/base.toml closes a cycle: ${TEMPLATE} -> ${FRAGMENT} -> ${TEMPLATE}`;
  runtime.editEntity.mockRejectedValueOnce(new Error(cycle));
  byId('add-include').value = BASE_HULL;
  byId('add-include-button').click();
  byId('apply-includes').click();
  await settled();
  expect(byId('status').getAttribute('role')).toBe('alert');
  expect(byId('status').textContent).toBe(t('workshop.entity.refused.cycle', { detail: cycle }));
  expect(document.activeElement).toBe(byId('status'));
  expect(draft.canUndo()).toBe(false);
  expect(draft.read(TEMPLATE)).toBe(templateSource);
  expect(changed).not.toHaveBeenCalled();
  expect(byId('apply-includes').disabled).toBe(false); // The reading is still current.
  expect(byId('includes').querySelectorAll('li')).toHaveLength(2); // The form keeps what was asked for.
  // wasm-bindgen may reject with a string; a materialise refusal maps the same way.
  runtime.materialiseEntity.mockRejectedValueOnce('materialise-local: hull.hull_integrity is already local');
  byId('field-1-materialise').click();
  await settled();
  expect(byId('status').textContent).toBe(t('workshop.entity.refused.local',
    { detail: 'materialise-local: hull.hull_integrity is already local' }));
  expect(draft.canUndo()).toBe(false);
  // An unrecognised refusal keeps the runtime's own words, which is all an author has.
  runtime.editEntity.mockRejectedValueOnce(new Error('the runtime is on fire'));
  byId('apply-includes').click();
  await settled();
  expect(byId('status').textContent).toBe(t('workshop.entity.refused.other', { detail: 'the runtime is on fire' }));
  // A draft that moved while the runtime was answering is not written over.
  let complete;
  runtime.editEntity.mockImplementationOnce(() => new Promise(resolve => { complete = resolve; }));
  byId('apply-includes').click();
  await vi.waitFor(() => expect(held).toBe(true));
  draft.edit(FRAGMENT, `${fragmentSource}# moved\n`);
  complete(`${templateSource}# late\n`);
  await settled();
  expect(draft.read(TEMPLATE)).toBe(templateSource);
  expect(byId('status').textContent).toBe(t('workshop.inspector_stale'));
  expect(changed).not.toHaveBeenCalled();
});

it('disables the forms once any text member changed and hides the panel under a Test hold', async () => {
  await read();
  draft.edit(FRAGMENT, `${fragmentSource}# newer\n`); panel.refresh();
  expect(byId('status').textContent).toBe(t('workshop.inspector_stale'));
  for (const id of ['include-0-remove', 'add-include', 'add-include-button', 'apply-includes', 'component-hull-remove',
    'component-comms-add', 'field-0-value', 'field-1-materialise', 'apply-fields', 'preview', 'test']) {
    expect(byId(id).disabled, id).toBe(true);
  }
  expect(byId('refresh').disabled).toBe(false);
  byId('apply-includes').click(); byId('apply-fields').click(); byId('component-comms-add').click();
  expect(runtime.editEntity).not.toHaveBeenCalled();
  draft.undo(); panel.refresh();
  expect(byId('apply-includes').disabled).toBe(false);
  expect(byId('preview').disabled).toBe(false);
  held = true; panel.refresh({ hidden: true });
  expect(section().hidden).toBe(true);
  for (const id of ['refresh', 'template', 'apply-includes', 'apply-fields', 'preview', 'test']) {
    expect(byId(id).disabled, id).toBe(true);
  }
  byId('apply-includes').click(); byId('refresh').click(); byId('preview').click();
  expect(runtime.editEntity).not.toHaveBeenCalled();
  expect(runtime.entity).toHaveBeenCalledTimes(1);
  expect(preview).not.toHaveBeenCalled();
  held = false; panel.refresh({ hidden: false });
  expect(section().hidden).toBe(false);
  expect(byId('apply-includes').disabled).toBe(false);
});

it('hands the composed template to the shared preview and to the existing Test, and says so when neither can take it', async () => {
  await read();
  byId('preview').click();
  expect(preview).toHaveBeenCalledExactlyOnceWith(TEMPLATE);
  expect(byId('status').textContent).toBe(t('workshop.entity.previewing', { path: TEMPLATE }));
  expect(byId('status').getAttribute('role')).toBe('status');
  byId('test').click();
  expect(test).toHaveBeenCalledExactlyOnceWith(TEMPLATE);
  expect(byId('status').textContent).toBe(t('workshop.entity.testing', { path: TEMPLATE }));
  preview.mockReturnValueOnce(false);
  byId('preview').click();
  expect(byId('status').textContent).toBe(t('workshop.entity.preview_unavailable', { detail: TEMPLATE }));
  expect(byId('status').getAttribute('role')).toBe('alert');
  test.mockReturnValueOnce(false);
  byId('test').click();
  expect(byId('status').textContent).toBe(t('workshop.entity.test_unavailable', { detail: TEMPLATE }));
  expect(byId('status').getAttribute('role')).toBe('alert');
  // Nothing is offered when the surface has no preview or Test to reach.
  panel.dispose();
  panel = mountWorkshopEntity({ root: document.getElementById('root'), runtime, draft: () => draft, win,
    busy: () => held, setBusy(value) { held = value; panel?.refresh(); }, changed });
  await read();
  expect(byId('preview').disabled).toBe(true);
  expect(byId('test').disabled).toBe(true);
});

it('says a draft with no entity template carries none rather than asking for one to be chosen', async () => {
  const empty = new WorkshopDocument(createStoreZip([{ path: MANIFEST, text: manifestSource }]));
  draft = empty; panel.refresh();
  expect(byId('template').options).toHaveLength(0);
  expect(byId('template').disabled).toBe(true);
  expect(byId('refresh').disabled).toBe(true);
  expect(byId('origin').textContent).toBe(t('workshop.entity.no_templates'));
  byId('refresh').click();
  expect(runtime.entity).not.toHaveBeenCalled();
});

it('does not display a reading that belongs to an older draft', async () => {
  let complete;
  runtime.entity.mockImplementationOnce(() => new Promise(resolve => { complete = resolve; }));
  byId('template').value = TEMPLATE;
  change(byId('template'));
  draft.edit(TEMPLATE, `${templateSource}# changed during read\n`);
  complete(composition());
  await settled();
  expect(byId('include-0-remove')).toBeNull();
  expect(byId('apply-includes').disabled).toBe(true);
});

it('labels every control, gives every button text and every group a legend, and keeps ids unique', async () => {
  await read();
  const controls = [...section().querySelectorAll('input, select')];
  expect(controls.length).toBeGreaterThan(4);
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
  // Ownership and severity are words in the text, never a colour alone.
  expect(byId('components').querySelector('[data-state="inherited"]').textContent).toContain(FRAGMENT);
  expect(byId('fields').querySelector('[data-owner="inherited"]').textContent)
    .toContain(t('workshop.entity.field_inherited', { source: FRAGMENT }));
  expect(byId('findings').querySelector('li').textContent).toContain(t('workshop.severity.error'));
});
