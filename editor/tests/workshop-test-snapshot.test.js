import { expect, it, vi } from 'vitest';
import { createWorkshopTestPreparation } from '../workshop-test-snapshot.js';
import { readStoreZipArchive } from '../mod-pack-export.js';
import { isWorkshopBinary } from '../workshop-document.js';

it('validates unsaved bytes before capture and merges immutable text/assets in the same ordered precedence', async () => {
  const runtime = { validate: vi.fn(async () => ({ accepted: true, findings: [] })),
    checkTestSelection: vi.fn(async () => ({ accepted: true, findings: [] })) };
  const dependencies = vi.fn(async () => ({ base_files: { 'assets/worlds/test.toml': 'base', 'assets/entities/ship.toml': 'hull' },
    base_assets: { 'assets/sounds/engine.wav': Uint8Array.of(1), 'assets/shaders/test.wgsl': Uint8Array.of(7) },
    packs: [{ files: { 'assets/worlds/test.toml': 'dependency' }, assets: { 'assets/sounds/engine.wav': Uint8Array.of(2) } }] }));
  const prepare = createWorkshopTestPreparation({ runtime, dependencies });
  const files = { 'scenarios.toml': '# manifest\r\n', 'assets/worlds/test.toml': '# unsaved\r\n',
    'assets/models/ship.glb': Uint8Array.of(0, 255) };
  const selection = { world: 'assets/worlds/test.toml', ship: 'assets/entities/ship.toml', seed: 7 };
  const captured = await prepare.prepare(files, selection);
  expect(readStoreZipArchive(runtime.validate.mock.calls[0][0], { binary: isWorkshopBinary }).files[selection.world]).toBe('# unsaved\r\n');
  expect(runtime.validate.mock.invocationCallOrder[0]).toBeLessThan(dependencies.mock.invocationCallOrder[0]);
  expect(captured.files).toMatchObject({ ...files, 'assets/entities/ship.toml': 'hull',
    'assets/sounds/engine.wav': Uint8Array.of(2), 'assets/shaders/test.wgsl': Uint8Array.of(7) });
  expect(runtime.checkTestSelection.mock.calls[0][0]).not.toHaveProperty('assets/models/ship.glb');
  expect((await prepare.prepare(files, selection)).revision).toBe(captured.revision);
});

it('preserves the validation report and performs no dependency capture after refusal', async () => {
  const report = { accepted: false, findings: [{ file: 'assets/worlds/test.toml', message: 'Rhai error' }] };
  const dependencies = vi.fn();
  const prepare = createWorkshopTestPreparation({ runtime: { validate: async () => report }, dependencies });
  await expect(prepare.prepare({ 'scenarios.toml': '' }, {})).rejects.toMatchObject({ report });
  expect(dependencies).not.toHaveBeenCalled();
});

it('uses textual read-only hull dependencies for catalog without fetching binary render content', async () => {
  const runtime = { dependencies: async () => ({ base_files: { 'assets/entities/base.toml': 'base' },
    packs: [{ files: { 'assets/entities/loaded.toml': 'loaded' } }] }), testCatalog: vi.fn(async files => ({ files })) };
  const dependencies = vi.fn();
  const prepare = createWorkshopTestPreparation({ runtime, dependencies });
  const catalog = await prepare.catalog({ 'assets/worlds/unsaved.toml': 'unsaved' });
  expect(catalog.files).toEqual({ 'assets/entities/base.toml': 'base', 'assets/entities/loaded.toml': 'loaded', 'assets/worlds/unsaved.toml': 'unsaved' });
  expect(dependencies).not.toHaveBeenCalled();
});
