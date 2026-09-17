import { test, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { createStoreZip, readStoreZipArchive } from '../../editor/mod-pack-export.js';
import { isWorkshopBinary } from '../../editor/workshop-document.js';
import { WORKSHOP_MANIFEST, WORKSHOP_WORLD, WORKSHOP_WORLD_TEXT } from '../fixtures/workshop-pack.js';
import { ts } from './strings';
import { revealWorkshopPanel } from './dock-helpers.js';

const MODEL = 'assets/models/workshop-rig.glb';
const RIG = 'assets/models/workshop-rig.model.toml';
const CLONE = 'assets/models/workshop-rig.weathered.toml';
// Deliberately integer-spelled runtime f32 fields, Unicode commentary and mixed
// line endings. Textarea values alone cannot prove this source was preserved.
const RIG_SOURCE = '# 原型 — keep the authored rig\r\n'
  + '[base] # correction\r\noffset = [1, 2, 3] # vector\n'
  + 'rotation = [0, 0, 0]\r\nscale = [1, 1, 1]\r\n'
  + '[markers.fore]\nposition = [0, 0, -1]\r\ndirection = [0, 0, -1]\r\n'
  + '[[target_points]]\r\nposition = [0, 0, 0]\n'
  + '[[lod]] # fallback\r\nshape = "sphere"\r\n'
  + '[lod.generate]\r\ntexture_size = 256 # unsigned runtime field\r\n';
const normaliseTextarea = source => source.replace(/\r\n?/g, '\n');

function triangleGlb() {
  const vertices = new Uint8Array(new Float32Array([0, 0, 0, 1, 0, 0, 0, 1, 0]).buffer);
  const definition = JSON.stringify({
    asset: { version: '2.0' },
    buffers: [{ byteLength: vertices.length }],
    bufferViews: [{ buffer: 0, byteLength: vertices.length }],
    accessors: [{ bufferView: 0, componentType: 5126, count: 3, type: 'VEC3', min: [0, 0, 0], max: [1, 1, 0] }],
    meshes: [{ primitives: [{ attributes: { POSITION: 0 }, mode: 4 }] }],
    nodes: [{ mesh: 0 }], scenes: [{ nodes: [0] }], scene: 0,
  });
  const json = new TextEncoder().encode(definition.padEnd(Math.ceil(definition.length / 4) * 4, ' '));
  const bytes = new Uint8Array(28 + json.length + vertices.length);
  const header = new DataView(bytes.buffer);
  header.setUint32(0, 0x46546c67, true); header.setUint32(4, 2, true);
  header.setUint32(8, bytes.length, true); header.setUint32(12, json.length, true);
  header.setUint32(16, 0x4e4f534a, true); bytes.set(json, 20);
  header.setUint32(20 + json.length, vertices.length, true);
  header.setUint32(24 + json.length, 0x004e4942, true);
  bytes.set(vertices, 28 + json.length);
  return bytes;
}

async function exportArchive(page) {
  const downloading = page.waitForEvent('download', { timeout: 90_000 });
  await page.locator('#workshop-export').click();
  const bytes = new Uint8Array(readFileSync(await (await downloading).path()));
  return { bytes, source: readStoreZipArchive(bytes, { binary: isWorkshopBinary }).source };
}

test('Workshop model fields use real runtime types and one exact-source history for edits and variants', { tag: '@core' }, async ({ page }, testInfo) => {
  test.setTimeout(180_000);
  const errors = [], sockets = [], wasm = [];
  page.on('pageerror', error => errors.push(error.message));
  page.on('websocket', socket => sockets.push(socket.url()));
  page.on('response', response => {
    if (new URL(response.url()).pathname.endsWith('.wasm') && response.ok()) wasm.push(response.url());
  });
  const model = triangleGlb();
  const archive = createStoreZip([
    { path: 'scenarios.toml', text: WORKSHOP_MANIFEST },
    { path: WORKSHOP_WORLD, text: WORKSHOP_WORLD_TEXT },
    { path: MODEL, bytes: model },
    { path: RIG, text: RIG_SOURCE },
  ]);
  await page.goto('/workshop.html');
  const importing = page.waitForEvent('filechooser');
  await page.locator('#workshop-import').click();
  await (await importing).setFiles({ name: 'model-rig.zip', mimeType: 'application/zip', buffer: Buffer.from(archive) });
  await page.locator('#workshop-files').selectOption(RIG);
  await expect(page.locator('#workshop-source')).toHaveValue(normaliseTextarea(RIG_SOURCE));
  await page.locator('#workshop-models > summary').click();
  await page.locator('#workshop-model').selectOption(MODEL);
  await page.locator('#workshop-model-variant').selectOption(RIG);
  await page.locator('#workshop-model-inspect').click();
  const offsetX = page.getByLabel('base.offset.[0]', { exact: true });
  const offsetY = page.getByLabel('base.offset.[1]', { exact: true });
  const textureSize = page.getByLabel('lod.[0].generate.texture_size', { exact: true });
  await expect(offsetX).toBeEnabled({ timeout: 90_000 });
  await expect(offsetX).toHaveValue('1');
  const metadata = offsetX.locator('..').locator('small');
  await expect(metadata).toContainText(ts('inspector.type', { type: 'float' }));
  await expect(metadata).toHaveAttribute('data-mutability', 'recreate-required');
  await expect(metadata).toContainText(ts('inspector.location', { path: RIG, line: '3' }));
  expect(wasm.length).toBeGreaterThan(0);

  // Two real wasm_workshop_patch calls produce one ordinary document edit.
  await offsetX.fill('1.5');
  await offsetY.fill('2.25');
  await page.locator('#workshop-model-apply').click();
  const edited = RIG_SOURCE.replace('[1, 2, 3]', '[1.5, 2.25, 3]');
  await expect(page.locator('#workshop-source')).toHaveValue(normaliseTextarea(edited));
  const changed = await exportArchive(page);
  expect(changed.source.entries.find(entry => entry.path === RIG).bytes).toEqual(new TextEncoder().encode(edited));
  expect(changed.source.entries.find(entry => entry.path === MODEL).bytes).toEqual(model);
  await page.locator('#workshop-undo').click();
  await expect(page.locator('#workshop-source')).toHaveValue(normaliseTextarea(RIG_SOURCE));
  await expect(page.locator('#workshop-undo')).toBeDisabled();
  expect((await exportArchive(page)).bytes).toEqual(archive);

  // A later invalid u32 must refuse the WHOLE grouped transaction, including
  // an earlier valid f32 patch. No partial source or undo entry may survive.
  await page.locator('#workshop-model-inspect').click();
  await expect(textureSize).toBeEnabled();
  await offsetX.fill('9.5');
  await textureSize.fill('-1');
  await page.locator('#workshop-model-apply').click();
  await expect(page.locator('#workshop-model-status')).toHaveText(ts('workshop.inspector_refused'));
  await expect(page.locator('#workshop-model-status')).toBeFocused();
  await expect(page.locator('#workshop-source')).toHaveValue(normaliseTextarea(RIG_SOURCE));
  await expect(page.locator('#workshop-undo')).toBeDisabled();

  await page.locator('#workshop-model-new-variant').fill('weathered');
  await page.locator('#workshop-model-clone').click();
  await expect(page.locator('#workshop-files')).toHaveValue(CLONE);
  await expect(page.locator('#workshop-model-variant')).toHaveValue(CLONE);
  await expect(page.locator('#workshop-source')).toHaveValue(normaliseTextarea(RIG_SOURCE));
  const cloned = await exportArchive(page);
  for (const path of [RIG, CLONE]) {
    expect(cloned.source.entries.find(entry => entry.path === path).bytes).toEqual(new TextEncoder().encode(RIG_SOURCE));
  }
  expect(cloned.source.entries.find(entry => entry.path === MODEL).bytes).toEqual(model);
  await page.locator('#workshop-model-new-variant').fill('weathered');
  await page.locator('#workshop-model-clone').click();
  await expect(page.locator('#workshop-model-status')).toHaveText(ts('workshop.models.variant_exists'));

  // A source edit outside the form invalidates its captured spans immediately.
  await page.locator('#workshop-model-inspect').click();
  await expect(offsetX).toBeEnabled();
  await revealWorkshopPanel(page, 'source');
  await page.locator('#workshop-source').fill(`${normaliseTextarea(RIG_SOURCE)}# newer source\n`);
  await expect(page.locator('#workshop-model-apply')).toBeDisabled();
  await expect(offsetX).toBeDisabled();
  await expect(page.locator('#workshop-model-status')).toHaveText(ts('workshop.inspector_stale'));
  await page.locator('#workshop-undo').click();
  await expect(page.locator('#workshop-source')).toHaveValue(normaliseTextarea(RIG_SOURCE));
  await page.locator('#workshop-undo').click();
  await expect(page.locator('#workshop-files option[value="' + CLONE + '"]')).toHaveCount(0);
  await expect(page.locator('#workshop-undo')).toBeDisabled();
  expect((await exportArchive(page)).bytes).toEqual(archive);
  expect(errors).toEqual([]);
  expect(sockets).toEqual([]);
  await page.screenshot({ path: testInfo.outputPath('workshop-models.png'), fullPage: true });
});
