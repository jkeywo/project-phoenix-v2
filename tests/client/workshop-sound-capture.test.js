import { readFileSync } from 'node:fs';
import { expect, it, vi } from 'vitest';
import { createWorkshopRuntime } from '../../editor/workshop-runtime.js';
import { createStoreZip, crc32 } from '../../editor/mod-pack-export.js';

const path = 'assets/sounds/custom/sonar ping.ogg';
const catalog = readFileSync(new URL('../fixtures/sound-cue-pack.toml', import.meta.url), 'utf8');
const sound = Uint8Array.from(readFileSync(new URL('../../assets/sounds/ui_click.ogg', import.meta.url)));
const candidate = () => createStoreZip([{ path: 'assets/audio/sound-cues.toml', text: catalog }]);
const response = bytes => ({ ok: true, arrayBuffer: async () => Uint8Array.from(bytes).buffer });

it('captures the Rust-discovered catalog sound from its declared immutable base before validation', async () => {
  const validate = vi.fn(() => JSON.stringify({ accepted: true, findings: [] }));
  const fetchAsset = vi.fn(async () => response(sound));
  const runtime = createWorkshopRuntime({
    dependencies: async () => ({ base_files: {}, packs: [],
      base_asset_manifest: { [path]: { length: sound.length, crc32: crc32(sound) } } }),
    load: async () => ({ wasm_workshop_asset_dependencies: () => [path], wasm_workshop_validate_pack: validate }),
    fetchAsset,
  });
  const bytes = candidate();
  await runtime.validate(bytes);
  expect(validate.mock.calls[0][0]).toBe(bytes);
  expect(JSON.parse(validate.mock.calls[0][1]).base_assets[path]).toEqual(Array.from(sound));
  expect(fetchAsset).toHaveBeenCalledExactlyOnceWith(path);
  const preview = await runtime.readAsset(path);
  expect(preview).toEqual(sound);
  preview.fill(0);
  await runtime.validate(bytes);
  expect(JSON.parse(validate.mock.lastCall[1]).base_assets[path]).toEqual(Array.from(sound));
  expect(fetchAsset).toHaveBeenCalledOnce();
});

it('keeps current dependency overrides and leaves missing or changed base sounds absent for the Rust gate', async () => {
  for (const mode of ['override', 'missing', 'changed']) {
    const validate = vi.fn(() => JSON.stringify({ accepted: false, findings: [] }));
    const fetchAsset = vi.fn(async () => response(new Uint8Array(sound.length)));
    const source = { base_files: {}, packs: mode === 'override'
      ? [{ assets: { [path]: [1, 2] } }, { assets: { [path]: Array.from(sound) } }] : [],
      base_asset_manifest: mode === 'missing' ? {} : { [path]: { length: sound.length, crc32: crc32(sound) } } };
    const runtime = createWorkshopRuntime({ dependencies: async () => source, fetchAsset,
      load: async () => ({ wasm_workshop_asset_dependencies: () => [path], wasm_workshop_validate_pack: validate }) });
    await runtime.validate(candidate());
    expect(JSON.parse(validate.mock.calls[0][1]).base_assets[path]).toBeUndefined();
    if (mode === 'override') {
      source.packs[1].assets[path].fill(0);
      expect(await runtime.readAsset(path)).toEqual(sound);
      expect(fetchAsset).not.toHaveBeenCalled();
    } else expect(fetchAsset).toHaveBeenCalledTimes(mode === 'missing' ? 0 : 1);
  }
});
