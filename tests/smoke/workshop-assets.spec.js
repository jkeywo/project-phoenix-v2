import { test, expect, openWorldPicker } from './fixtures';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { createStoreZip, readStoreZipArchive } from '../../editor/mod-pack-export.js';
import { isWorkshopBinary } from '../../editor/workshop-document.js';
import { WORKSHOP_MANIFEST, WORKSHOP_WORLD, WORKSHOP_WORLD_TEXT } from '../fixtures/workshop-pack.js';
import { ts } from './strings';
import { revealWorkshopPanel } from './dock-helpers.js';

const MODEL = 'assets/models/workshop-triangle.glb';
const BUFFER = 'assets/models/workshop-vertices.bin';
const IMAGE = 'assets/models/workshop-image.png';
const SOUND = 'assets/sounds/workshop-click.ogg';
const content = relative => new Uint8Array(readFileSync(path.join(__dirname, '../..', relative)));
const vertices = new Uint8Array(new Float32Array([0, 0, 0, 1, 0, 0, 0, 1, 0]).buffer);
const model = (() => {
  const definition = JSON.stringify({
    asset: { version: '2.0' },
    buffers: [{ uri: 'workshop-vertices.bin', byteLength: vertices.length }],
    bufferViews: [{ buffer: 0, byteLength: vertices.length }],
    accessors: [{ bufferView: 0, componentType: 5126, count: 3, type: 'VEC3', min: [0, 0, 0], max: [1, 1, 0] }],
    meshes: [{ primitives: [{ attributes: { POSITION: 0 }, mode: 4 }] }],
    images: [{ uri: 'workshop-image.png' }],
    nodes: [{ mesh: 0 }], scenes: [{ nodes: [0] }], scene: 0,
  });
  const json = new TextEncoder().encode(definition.padEnd(Math.ceil(definition.length / 4) * 4, ' '));
  const bytes = new Uint8Array(20 + json.length);
  const header = new DataView(bytes.buffer);
  header.setUint32(0, 0x46546c67, true); header.setUint32(4, 2, true);
  header.setUint32(8, bytes.length, true); header.setUint32(12, json.length, true);
  header.setUint32(16, 0x4e4f534a, true); bytes.set(json, 20);
  return bytes;
})();
const binaryEntries = () => [
  { path: MODEL, bytes: model }, { path: BUFFER, bytes: vertices },
  { path: IMAGE, bytes: content('assets/logo.png') },
  { path: SOUND, bytes: content('assets/sounds/ui_click.ogg') },
];
const archive = () => createStoreZip([
  { path: 'scenarios.toml', text: WORKSHOP_MANIFEST },
  { path: WORKSHOP_WORLD, text: WORKSHOP_WORLD_TEXT },
  ...binaryEntries(),
]);

test('Workshop validates real model, image and audio bytes, recovers a refused buffer and exports the same bytes to the host', { tag: '@core' }, async ({ page, context }) => {
  test.setTimeout(180_000);
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  page.on('dialog', dialog => dialog.accept());
  await page.goto('/workshop.html');
  const importing = page.waitForEvent('filechooser');
  await page.locator('#workshop-import').click();
  await (await importing).setFiles({ name: 'binary-source.zip', mimeType: 'application/zip', buffer: Buffer.from(archive()) });
  await page.locator('#workshop-files').selectOption(BUFFER);
  await expect(page.locator('#workshop-source')).toBeDisabled();
  // A real external buffer passes through the same authoring history as text.
  // Its broken replacement is retained for repair, but runtime export refuses it.
  await revealWorkshopPanel(page, 'add');
  await page.locator('#workshop-add-path').fill(BUFFER);
  const replacing = page.waitForEvent('filechooser');
  await page.locator('#workshop-add-asset').click();
  await (await replacing).setFiles({ name: 'workshop-vertices.bin', mimeType: 'application/octet-stream', buffer: Buffer.from([255]) });
  await page.locator('#workshop-check').click();
  await expect(page.locator('.workshop-findings[role="alert"]')).toContainText(ts('workshop.check_refused'), { timeout: 90_000 });
  await expect(page.locator('.workshop-findings[role="alert"]')).toContainText(MODEL);
  await expect(page.locator('#workshop-recovery-status')).toHaveText(ts('workshop.recovery_saved'));
  await page.reload();
  await revealWorkshopPanel(page, 'recovery');
  await page.locator('#workshop-restore').click();
  await page.locator('#workshop-undo').click();
  const downloading = page.waitForEvent('download', { timeout: 90_000 });
  await page.locator('#workshop-export').click();
  const downloaded = new Uint8Array(readFileSync(await (await downloading).path()));
  expect(downloaded).toEqual(archive());
  const exported = readStoreZipArchive(downloaded, { binary: isWorkshopBinary });
  for (const entry of binaryEntries()) {
    expect(exported.source.entries.find(value => value.path === entry.path).bytes).toEqual(entry.bytes);
  }
  const host = await context.newPage();
  const hostErrors = [];
  host.on('pageerror', error => hostErrors.push(error.message));
  host.on('console', message => {
    if (message.type() === 'error') hostErrors.push(message.text());
  });
  await host.goto('/'); await openWorldPicker(host);
  await expect(host.locator('#world-list .world-btn[data-scenario-id]').first()).toBeVisible();
  await host.locator('#mod-pack-file').setInputFiles({ name: 'binary-export.zip', mimeType: 'application/zip', buffer: Buffer.from(downloaded) });
  try {
    await expect(host.locator('#mod-pack-status')).toContainText(ts('server.mod_pack_applied'));
  } catch (error) {
    await test.info().attach('host-errors', { body: hostErrors.join('\n'), contentType: 'text/plain' });
    throw new Error(`${error.message}\nHost errors:\n${hostErrors.join('\n')}`, { cause: error });
  }
  // Inspect the accepted provider, not the uploaded file or a network fixture.
  const retained = await host.evaluate(paths => Object.fromEntries(paths.map(assetPath => [assetPath, Array.from(window.__hostReadPackAsset(assetPath) || [])])), binaryEntries().map(value => value.path));
  for (const entry of binaryEntries()) expect(retained[entry.path]).toEqual(Array.from(entry.bytes));
  expect(errors).toEqual([]);
});
