import { test, expect } from '@playwright/test';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { WORKSHOP_MANIFEST } from '../fixtures/workshop-pack.js';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { revealWorkshopPanel } from './dock-helpers.js';

// A model that exists ONLY in the imported draft. The whole point of the
// assertion below is that pixels come from bytes the project does not have:
// if the preview were reading the repo instead of its capture, a draft-only
// path would render nothing at all.
const DRAFT_MODEL = 'assets/models/workshop_preview_only.glb';
// Resolved from this file, not the working directory: playwright runs specs
// with tests/smoke as its cwd. `__dirname` rather than `import.meta.url`
// because Playwright transpiles these specs to CommonJS, where import.meta is
// not available — the rest of tests/smoke resolves paths the same way.
const SHIPPED_MODEL = path.join(__dirname, '../../assets/models/alliance_cruiser.glb');

const isPreviewFrame = frame =>
  /\/preview\/(index\.html)?$/.test(new URL(frame.url() || 'about:blank').pathname);

const sourcePack = () => createStoreZip([
  { path: 'scenarios.toml', text: WORKSHOP_MANIFEST },
  { path: 'assets/worlds/workshop.toml', text: '[global]\n[anchors]\n' },
  // The draft carries a real GLB under a name the repo has never heard of.
  { path: DRAFT_MODEL, bytes: new Uint8Array(readFileSync(SHIPPED_MODEL)) },
]);

/** How many distinct colours the preview canvas is showing.
 *
 * One colour means the renderer cleared and drew nothing — which is exactly
 * what a missing asset looks like, and why this is the assertion that proves
 * the captured bytes actually reached the render path.
 */
async function previewColours(page, frame) {
  const box = await frame.locator('#canvas').boundingBox();
  expect(box).toBeTruthy();
  const png = await page.screenshot({ clip: {
    x: Math.round(box.x + box.width * 0.25), y: Math.round(box.y + box.height * 0.25),
    width: Math.round(box.width * 0.5), height: Math.round(box.height * 0.5) } });
  // Decoded in the host page: a FrameLocator addresses a frame, it does not
  // evaluate in one, and the pixels are already in the screenshot.
  return page.evaluate(async value => {
    const image = new Image(); image.src = `data:image/png;base64,${value}`; await image.decode();
    const canvas = document.createElement('canvas');
    canvas.width = image.width; canvas.height = image.height;
    const context = canvas.getContext('2d'); context.drawImage(image, 0, 0);
    const pixels = context.getImageData(0, 0, canvas.width, canvas.height).data;
    const colours = new Set();
    for (let i = 0; i < pixels.length && colours.size < 64; i += 4) {
      colours.add((pixels[i] << 16) | (pixels[i + 1] << 8) | pixels[i + 2]);
    }
    return colours.size;
  }, png.toString('base64'));
}

test('the Workshop preview renders a draft-only model through the shared viewer, reading nothing else',
  { tag: '@core' }, async ({ page }) => {
    test.setTimeout(240_000);
    const errors = [];
    const projectAssetRequests = [];
    page.on('pageerror', error => errors.push(error.message));
    // Anything the PREVIEW frame asks the server for under /assets/ would be a
    // read outside its capture. The frame should make none.
    page.on('request', request => {
      if (isPreviewFrame(request.frame()) && new URL(request.url()).pathname.startsWith('/assets/')) {
        projectAssetRequests.push(request.url());
      }
    });
    await page.addInitScript(() => {
      Object.defineProperty(navigator, 'webdriver', { get: () => false });
    });

    await page.goto('/workshop.html');
    const chooser = page.waitForEvent('filechooser');
    await page.locator('#workshop-import').click();
    await (await chooser).setFiles({
      name: 'preview-draft.zip', mimeType: 'application/zip', buffer: Buffer.from(sourcePack()),
    });

    await revealWorkshopPanel(page, 'models');
    await page.locator('#workshop-model').selectOption(DRAFT_MODEL);
    await revealWorkshopPanel(page, 'model-preview');
    await expect(page.locator('#workshop-model-preview-refresh')).toBeEnabled();
    await page.locator('#workshop-model-preview-refresh').click();

    const frame = page.frameLocator('.workshop-model-preview-frame');
    // Statistics are measured by the shared ViewerPlugin, so a settled reading
    // with triangles in it is the viewer's own count of the captured mesh.
    await expect.poll(async () => await page.locator('#workshop-model-preview-stats').textContent(),
      { timeout: 180_000 }).toMatch(/\d/);

    const colours = await previewColours(page, frame);
    expect(colours, 'the preview drew a scene rather than clearing to one colour').toBeGreaterThan(1);

    // And it drew it from the capture: the frame asked the server for no
    // project asset at all.
    expect(projectAssetRequests).toEqual([]);
    expect(errors).toEqual([]);
  });
