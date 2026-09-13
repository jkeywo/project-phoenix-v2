// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { mountWorkshopModelPreview } from '../../gui/workshop-model-preview-panel.js';
import { t } from '../../gui/strings.js';

let panel, backend, draft, selection, held, current;
const get = name => document.getElementById(`workshop-model-preview-${name}`);
beforeEach(() => {
  document.body.innerHTML = '<main id="root"></main>';
  draft = { sourceRevision: 1 }; selection = { model: 'assets/models/test.glb', variant: 'model' }; held = false;
  current = { running: true, starting: false, acknowledged: 0, revision: 'one', selection, error: null,
    stats: { triangles: 14, meshes: 2, textures: 3, measured_textures: 2, texture_pixels: 256, largest_texture: 16,
      distance: 10, levels: 2, level: null, mode: 'auto', settled: true,
      camera: { focus: [1, 2, 3], radius: 10, yaw: 1, pitch: 0 } } };
  backend = { mount: vi.fn(), capture: vi.fn(value => ({ revision: value.sourceRevision })),
    start: vi.fn(async () => structuredClone(current)), control: vi.fn(async () => structuredClone(current)),
    status: vi.fn(async () => structuredClone(current)), stop: vi.fn(async () => {}) };
  panel = mountWorkshopModelPreview({ root: document.getElementById('root'), provider: { modelPreview: backend },
    draft: () => draft, selection: () => selection, busy: () => held });
});
afterEach(() => panel.dispose());

it('previews only after Refresh and sends camera and LOD controls to the local provider', async () => {
  expect(backend.start).not.toHaveBeenCalled();
  get('refresh').click();
  await vi.waitFor(() => expect(get('left').disabled).toBe(false));
  expect(backend.capture).toHaveBeenCalledWith(draft);
  expect(backend.start).toHaveBeenCalledWith({ revision: 1 }, selection);
  expect(get('stats').textContent).toContain('14');
  expect(get('lod').options.length).toBe(4);
  get('left').click();
  await vi.waitFor(() => expect(backend.control).toHaveBeenCalledWith({ command: 'camera', focus: [1, 2, 3], radius: 10, yaw: 1 - Math.PI / 12, pitch: 0 }));
  get('lod').value = 'fixed:1'; get('lod').dispatchEvent(new Event('change'));
  await vi.waitFor(() => expect(backend.control).toHaveBeenCalledWith({ command: 'lod', mode: 'fixed', level: 1 }));
  expect(draft).toEqual({ sourceRevision: 1 });
});

it('marks the previous picture stale after edits and retires it on Test or draft replacement', async () => {
  get('refresh').click(); await vi.waitFor(() => expect(get('stop').disabled).toBe(false));
  draft.sourceRevision++; panel.refresh();
  expect(get('status').textContent).toContain(t('workshop.models.preview.stale'));
  expect(backend.start).toHaveBeenCalledTimes(1);
  held = true; panel.refresh({ hidden: true });
  expect(get('panel').hidden).toBe(true); expect(backend.stop).toHaveBeenCalledTimes(1);
  held = false; panel.refresh(); get('refresh').click();
  await vi.waitFor(() => expect(backend.start).toHaveBeenCalledTimes(2));
  draft = { sourceRevision: 0 }; panel.refresh();
  expect(backend.stop).toHaveBeenCalledTimes(2);
});

it('reports a refused preview safely without changing source or losing deliberate retry', async () => {
  backend.start.mockRejectedValueOnce(new Error('<script>missing rig</script>'));
  get('refresh').click();
  await vi.waitFor(() => expect(get('status').getAttribute('role')).toBe('alert'));
  expect(get('status').textContent).toContain('<script>missing rig</script>');
  expect(get('status').querySelector('script')).toBeNull(); expect(get('refresh').disabled).toBe(false);
  expect(draft.sourceRevision).toBe(1); expect(backend.control).not.toHaveBeenCalled();
});

it('exposes unavailable preview honestly without a provider or fabricated controls', () => {
  panel.dispose();
  panel = mountWorkshopModelPreview({ root: document.getElementById('root'), draft: () => draft,
    selection: () => selection, busy: () => false });
  expect(get('refresh').disabled).toBe(true);
  expect(get('status').textContent).toBe(t('workshop.models.preview.unavailable'));
});
