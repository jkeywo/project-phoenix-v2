// #1248: real local-slot/import gates wait for static sibling content and
// composed literal hulls before checking the existing save identity.
import { test, expect, MINIMAL_DEFAULT_WORLD, waitForWasmReady,
  readHostPeerId, createTestClient, openSaveCatalogue } from './fixtures';

const WORLD = 'assets/worlds/default.toml';
const CHILD = 'assets/worlds/preinit-child.toml';
const SCRIPT = 'assets/worlds/preinit-child.rhai';
const HULL = 'assets/entities/preinit-only.toml';
const FRAGMENT = 'assets/entities/preinit-fragment.toml';
const ROOT = `script = "preinit-root.rhai"\nextra_worlds = ["${CHILD}"]\n`
  + MINIMAL_DEFAULT_WORLD.replace(/\[script\][\s\S]*$/, '');
const SOURCE = `fn unused(ctx) { ctx.effects.spawn_entity(#{ template_path: "${HULL}" }); }`;

async function install(context, state) {
  const files = () => ({
    [WORLD]: ROOT + (state.promotion ? `\n[[entity]]\ntemplate_path = "${HULL}"\nname = "Include owner"\n` : ''),
    [CHILD]: 'script = "preinit-child.rhai"\n[global]\n',
    'assets/worlds/preinit-root.rhai': 'fn root() {}',
    [SCRIPT]: (state.promotion ? SOURCE.replace(HULL, FRAGMENT) : SOURCE)
      + (state.changed === 'script' ? '\n// edited sibling' : ''),
    [HULL]: `includes = ["${FRAGMENT}"]\nname = "Sibling-only hull"\n`,
    [FRAGMENT]: `mass = ${state.changed === 'hull' ? '2.0' : '1.0'}\n`,
  });
  for (const path of Object.keys(files())) {
    await context.route(`**/${path}`, async route => {
      if (path.endsWith('.rhai')) {
        const slow = (path === SCRIPT) !== !!state.reverse;
        await new Promise(resolve => setTimeout(resolve, slow ? 300 : 25));
      }
      if (state.promotion && state.reverse && path === HULL) {
        await new Promise(resolve => setTimeout(resolve, 500));
      }
      if (state.missing === path) return route.fulfill({ status: 404, body: 'missing' });
      await route.fulfill({ contentType: 'text/plain', body: files()[path] });
    });
  }
}

async function savedHost(context) {
  const page = await context.newPage();
  await page.goto(`/?scenario=${WORLD}`);
  await waitForWasmReady(page);
  const client = await createTestClient(context, await readHostPeerId(page), { name: 'Preinit tester' });
  await client.send('SelectStation', { station: 'Captain' });
  await client.page.waitForFunction(token => window.__messages?.some(m =>
    m.type === 'StationAssigned' && m.data.token === token), client.token);
  await client.send('SetReady', { ready: true });
  await client.waitForMessage('GameStarted', 20000);
  await page.bringToFront();
  const panel = page.locator('#manual-save-panel');
  await expect(panel.locator('[data-save-action="create"]')).toBeEnabled({ timeout: 20000 });
  await panel.locator('input').fill('Preinit content');
  await panel.locator('[data-save-action="create"]').click();
  await page.waitForFunction(() => Array.from(window.wasm_list_save_slots?.() || [])
    .some(row => row.display_name === 'Preinit content'));
  const save = await page.evaluate(() => {
    const row = Array.from(window.wasm_list_save_slots()).find(row => row.display_name === 'Preinit content');
    const refusal = window.wasm_export_save_slot(row.slot_id);
    if (refusal) throw Error(refusal);
    return { slot: row.slot_id, text: window.wasm_take_exported_snapshot() };
  });
  await client.close();
  return { page, ...save };
}

for (const mode of ['local slot', 'import']) {
  for (const changed of [null, 'script', 'hull', 'promotion']) {
    test(`${mode} ${changed === 'promotion' ? 'accepts a fragment promoted to a template in either fetch order' : changed ? `refuses changed sibling ${changed}` : 'accepts unchanged static sibling content'}`, async ({ context }) => {
      test.setTimeout(180000);
      const state = { promotion: changed === 'promotion' };
      await install(context, state);
      const saved = await savedHost(context);
      state.changed = changed;
      state.reverse = true;
      await saved.page.evaluate(() => window.phSaveSlots.armBrowserSaveIdentityHandoff(window));
      await saved.page.goto('/');
      await openSaveCatalogue(saved.page);
      if (mode === 'local slot') {
        await saved.page.locator(`[data-slot-id="${saved.slot}"]`).click();
        await saved.page.locator('[data-save-action="start"]').click();
      } else {
        // The catalogue shell is first-paint HTML. Wait for WASM catalogue
        // population before sending a file to its guarded change handler.
        await saved.page.locator('#world-list .world-btn[data-scenario-id]').first()
          .waitFor({ state: 'attached', timeout: 30000 });
        await saved.page.locator('#snapshot-import-file').setInputFiles({
          name: 'preinit.ron', mimeType: 'text/plain', buffer: Buffer.from(saved.text),
        });
      }
      if (changed && changed !== 'promotion') {
        await expect(saved.page.locator('#snapshot-status')).toContainText('authored data has changed', { timeout: 60000 });
        expect(await saved.page.evaluate(() => !!window.wasm_resume_pending?.())).toBe(false);
      } else {
        await saved.page.waitForFunction(() => window.wasm_resume_pending?.(), null, { timeout: 60000 });
      }
    });
  }
}

for (const missing of [SCRIPT, HULL, FRAGMENT]) {
  test(`missing ${missing} refuses before App start`, async ({ context }) => {
    await install(context, { missing });
    const page = await context.newPage();
    await page.goto(`/?scenario=${WORLD}`);
    await expect(page.locator('#wasm-spinner')).toContainText(missing, { timeout: 60000 });
    expect(await page.evaluate(() => !!window.__wasmReady)).toBe(false);
  });
}
