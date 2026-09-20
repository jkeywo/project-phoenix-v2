// @vitest-environment jsdom
import { beforeEach, afterEach, it, expect, vi } from 'vitest';
import { WorkshopDocument } from '../../editor/workshop-document.js';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { mountWorkshopModels } from '../../gui/workshop-models-panel.js';
import { t } from '../../gui/strings.js';

const path = 'assets/models/ship.model.toml';
const source = '# origin\r\n[base]\r\noffset = [1, 2, 3] # keep\r\n\r\n[markers.muzzle] # keep marker\r\nposition = [0, 0, 0]\r\ndirection = [0, 0, 1]\r\n';
const fields = [0, 1, 2].map(index => ({ path: ['base', 'offset', index], kind: 'float',
  source: String(index + 1), line: 3, runtime_owned: false, default_source: null }));
let draft, runtime, panel, held, changed;
const byId = id => document.getElementById(`workshop-model-${id}`);
beforeEach(() => {
  document.body.innerHTML = '<main id="root"></main>';
  draft = new WorkshopDocument(createStoreZip([{ path, text: source },
    { path: 'assets/models/ship.glb', bytes: new Uint8Array([1, 2, 3]) }]), { kind: 'project' });
  held = false;
  runtime = { dependencies: vi.fn(async () => ({ base_files: {}, base_asset_manifest: {}, packs: [] })),
    validate: vi.fn(async () => ({ accepted: true, findings: [] })),
    inspect: vi.fn(async () => fields), patch: vi.fn(async (text, patch) => {
    if (patch.path[2] === 0) return text.replace('[1, 2, 3]', `[${patch.value_source}, 2, 3]`);
    return text.replace(', 2, 3]', `, ${patch.value_source}, 3]`);
  }) };
  changed = vi.fn();
  panel = mountWorkshopModels({ root: document.getElementById('root'), runtime, draft: () => draft,
    busy: () => held, setBusy(value) { held = value; panel?.refresh(); }, changed });
});
afterEach(() => panel.dispose());

it('shows grouped current source fields and applies a vector as one reversible change', async () => {
  byId('inspect').click();
  await vi.waitFor(() => expect(byId('field-0')).not.toBeNull());
  expect(document.querySelector('.workshop-model-fields legend').textContent).toBe(t('workshop.models.group.base'));
  expect(document.querySelector('label[for="workshop-model-field-0"]').textContent).toBe('base.offset.[0]');
  byId('field-0').value = '8'; byId('field-1').value = '9';
  byId('apply').click();
  await vi.waitFor(() => expect(changed).toHaveBeenCalledWith(path));
  expect(draft.read(path)).toBe(source.replace('[1, 2, 3]', '[8, 9, 3]'));
  expect(draft.undo()).toBe(path); expect(draft.read(path)).toBe(source); expect(draft.canUndo()).toBe(false);
});

it('refuses overwrite and clones source as an ordinary new variant without a filesystem capability', async () => {
  byId('new-variant').value = 'model'; byId('clone').click();
  expect(byId('status').getAttribute('role')).toBe('alert');
  expect(byId('status').textContent).toBe(t('workshop.models.variant_exists'));
  expect(draft.canUndo()).toBe(false);
  byId('new-variant').value = 'damaged'; byId('clone').click();
  await vi.waitFor(() => expect(draft.read('assets/models/ship.damaged.toml')).toBe(source));
  expect(byId('variant').value).toBe('assets/models/ship.damaged.toml');
  expect(changed).toHaveBeenCalledTimes(1);
});

it('can remove the last sidecar and create a new identity variant for the captured GLB', async () => {
  byId('remove-variant').click();
  await vi.waitFor(() => expect(draft.read(path)).toBeUndefined());
  expect(byId('variant').options).toHaveLength(0); expect(byId('clone').disabled).toBe(false);
  byId('new-variant').value = 'replacement'; byId('clone').click();
  const replacement = 'assets/models/ship.replacement.toml';
  await vi.waitFor(() => expect(draft.read(replacement)).toBe(''));
  expect(byId('variant').value).toBe(replacement);
  expect(runtime.validate).toHaveBeenCalledTimes(2);
});

it('offers keyboard controls for exact marker CRUD and reports its source owner', async () => {
  await vi.waitFor(() => expect(document.querySelector('#workshop-model-markers .workshop-model-owner')).not.toBeNull());
  const owner = document.querySelector('#workshop-model-markers .workshop-model-owner');
  expect(owner.textContent).toContain(`${path}`); expect(owner.textContent).toContain('line 5');
  const name = byId('marker-new-name'); name.value = 'dock';
  byId('marker-new-position').value = '1, 2, 3'; byId('marker-new-direction').value = '0, 1, 0';
  const add = byId('marker-add'); add.focus(); expect(document.activeElement).toBe(add); add.click();
  await vi.waitFor(() => expect(draft.read(path)).toContain('[markers.dock]'));
  expect(draft.read(path)).toContain('[markers.muzzle] # keep marker');
  expect(runtime.validate).toHaveBeenCalled(); expect(changed).toHaveBeenCalledWith(path);
});

it('holds controls in Test and disables stale readings after a source edit', async () => {
  byId('inspect').click();
  await vi.waitFor(() => expect(byId('field-0')).not.toBeNull());
  held = true; panel.refresh({ hidden: true });
  expect(document.getElementById('workshop-models').hidden).toBe(true);
  expect(byId('field-0').disabled).toBe(true);
  byId('apply').click(); byId('clone').click();
  expect(runtime.patch).not.toHaveBeenCalled(); expect(draft.canUndo()).toBe(false);
  held = false; draft.edit(path, `${source}# newer\r\n`); panel.refresh();
  expect(byId('field-0').disabled).toBe(true); expect(byId('apply').disabled).toBe(true);
  expect(byId('status').textContent).toBe(t('workshop.inspector_stale'));
});

it('does not display an inspector response that belongs to an older draft', async () => {
  let complete;
  runtime.inspect.mockImplementation(() => new Promise(resolve => { complete = resolve; }));
  byId('inspect').click();
  draft.edit(path, `${source}# changed during read\r\n`);
  complete(fields);
  await vi.waitFor(() => expect(held).toBe(false));
  expect(byId('field-0')).toBeNull(); expect(byId('apply').disabled).toBe(true);
});

it('stops showing the captured preview when its own panel stops being shown', () => {
  const preview = () => document.getElementById('workshop-model-preview-panel');
  expect(preview().hidden).toBe(false);
  panel.setPreviewVisible(false);
  expect(preview().hidden).toBe(true);
  panel.setPreviewVisible(true);
  expect(preview().hidden).toBe(false);
  // Test mode still hides it regardless of the panel's own visibility.
  held = true; panel.refresh({ hidden: true });
  expect(preview().hidden).toBe(true);
});

it('applies a migrated model selection through the ordinary Models controls', () => {
  expect(panel.applyLaunch({ model: 'assets/models/ship.glb', variant: 'model' },
    { lighting: 'ambient', gizmos: false })).toBe(true);
  expect(document.getElementById('workshop-model').value).toBe('assets/models/ship.glb');
  expect(byId('variant').value).toBe(path);
  expect(document.getElementById('workshop-model-preview-lighting').value).toBe('ambient');
  expect(document.getElementById('workshop-model-preview-gizmos').checked).toBe(false);
  expect(panel.applyLaunch({ model: 'assets/models/missing.glb' })).toBe(false);
});

it('cancels a native capture immediately when the selected variant changes', async () => {
  panel.dispose();
  const captureSource = '[[lod]]\nmodel="assets/models/ship.glb"\nmax_distance=10\n[[lod]]\nbillboard="assets/models/ship.png"\n[lod.capture]\nsource="assets/models/ship.glb"\nyaw_views=8\nresolution=64\npitch=20\n';
  draft = WorkshopDocument.fromNativeFiles({
    [path]: captureSource,
    'assets/models/ship.alt.toml': captureSource,
    'assets/models/ship.glb': { asset: '0000000000000001-3', length: 3 },
  }, { kind: 'project' });
  const billboardCapture = {
    active: null,
    start: vi.fn(async () => {
      billboardCapture.active = { state: 'running', sidecar: path, lod: 1 };
      return billboardCapture.active;
    }),
    cancel: vi.fn(async () => { billboardCapture.active = null; }),
  };
  panel = mountWorkshopModels({ root: document.getElementById('root'), provider: { billboardCapture }, runtime,
    draft: () => draft, busy: () => held, setBusy(value) { held = value; panel?.refresh(); }, changed });
  await vi.waitFor(() => expect(document.getElementById('workshop-model-capture-start-1')).not.toBeNull());
  document.getElementById('workshop-model-capture-start-1').click();
  await vi.waitFor(() => expect(billboardCapture.start).toHaveBeenCalled());
  const variants = byId('variant'); variants.value = 'assets/models/ship.alt.toml';
  variants.dispatchEvent(new Event('change'));
  await vi.waitFor(() => expect(billboardCapture.cancel).toHaveBeenCalled());
});

it('exposes labelled native generation and optional remesh controls only when the capability and recipe exist', async () => {
  panel.dispose();
  const generated = 'assets/models/ship_lod1.glb';
  draft.put(generated, Uint8Array.of(4, 5, 6));
  draft.edit(path, `${source}\r\n[[lod]]\r\nmodel = "${generated}"\r\n\r\n[lod.generate]\r\nsource = "assets/models/ship.glb"\r\nratio = 0.5\r\n`);
  const lodGeneration = { active: null, start: vi.fn(async () => ({ sidecar: path, progress: [] })),
    status: vi.fn(), reviewDraft: vi.fn(), adopt: vi.fn(), cancel: vi.fn(async () => {}) };
  panel = mountWorkshopModels({ root: document.getElementById('root'), provider: { lodGeneration }, runtime, draft: () => draft,
    busy: () => held, setBusy(value) { held = value; panel?.refresh(); }, changed });
  document.getElementById('workshop-model').value = 'assets/models/ship.glb';
  document.getElementById('workshop-model').dispatchEvent(new Event('change'));
  await vi.waitFor(() => expect(document.getElementById('workshop-model-generation')).not.toBeNull());
  const remesh = document.getElementById('workshop-model-generation-remesh');
  expect(remesh.type).toBe('checkbox');
  expect(document.querySelector(`label[for="${remesh.id}"]`).textContent).toBe(t('workshop.models.generation.remesh'));
  expect(document.getElementById('workshop-model-generation-progress').getAttribute('aria-live')).toBe('polite');
  document.getElementById('workshop-model-generation-start').click();
  await vi.waitFor(() => expect(lodGeneration.start).toHaveBeenCalledWith(draft, path, { remesh: false }));
});
