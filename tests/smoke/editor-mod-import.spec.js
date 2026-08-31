// Issue #1321 — golden browser workflow for the bounded T2 MOD-editor import.
//
// The deployed host intentionally does not publish the legacy editor.  This
// spec routes the checkout's real editor/gui modules into one isolated page,
// keeping the test a real Chromium file-chooser workflow without expanding the
// production dist surface that M6 will later replace.

import { test, expect, modPackFixturePath } from './fixtures';
import fs from 'fs';
import path from 'path';

const ROOT = path.resolve(__dirname, '../..');
const SMOL_DIST = [
  path.join(ROOT, 'node_modules/smol-toml/dist'),
  // Local issue worktrees share the parent checkout's installed dependencies.
  path.resolve(ROOT, '../../node_modules/smol-toml/dist'),
].find((candidate) => fs.existsSync(candidate));
if (!SMOL_DIST) throw new Error('smol-toml browser modules are not installed');
const ROUTES = [
  ['/__editor/', path.join(ROOT, 'editor')],
  ['/gui/', path.join(ROOT, 'gui')],
  ['/assets/', path.join(ROOT, 'assets')],
  ['/__smol/', SMOL_DIST],
];

function contentType(filename) {
  if (filename.endsWith('.js')) return 'text/javascript; charset=utf-8';
  if (filename.endsWith('.css')) return 'text/css; charset=utf-8';
  if (filename.endsWith('.csv')) return 'text/csv; charset=utf-8';
  return 'application/octet-stream';
}

async function routeCheckoutModules(context) {
  for (const [prefix, localRoot] of ROUTES) {
    await context.route(`**${prefix}**`, async (route) => {
      const pathname = decodeURIComponent(new URL(route.request().url()).pathname);
      const candidate = path.resolve(localRoot, pathname.slice(prefix.length));
      const inside = candidate === localRoot || candidate.startsWith(`${localRoot}${path.sep}`);
      if (!inside || !fs.existsSync(candidate) || !fs.statSync(candidate).isFile()) {
        await route.abort();
        return;
      }
      await route.fulfill({
        path: candidate,
        contentType: contentType(candidate),
      });
    });
  }

  await context.route('**/__editor-mod-import.html', async (route) => {
    await route.fulfill({
      contentType: 'text/html; charset=utf-8',
      body: `<!doctype html>
        <html lang="en">
          <head>
            <meta charset="utf-8">
            <script type="importmap">
              {"imports":{"smol-toml":"/__smol/index.js"}}
            </script>
            <link rel="stylesheet" href="/__editor/style.css">
          </head>
          <body>
            <main id="mod-mode-root"></main>
            <script type="module">
              try {
              // The production editor's strings boot uses synchronous XHR so
              // its module graph is ready before the page load event.  A
              // Playwright request route cannot answer a synchronous browser
              // request without deadlocking, so this isolated fixture marks
              // that boot as test-owned and installs the same real CSV first.
              window.process = { env: { VITEST: 'true' } };
              const { buildTable, setTable } = await import('/gui/strings.js');
              const csv = await (await fetch('/assets/strings/strings.csv')).text();
              setTable(buildTable(csv));
              const { ModeShell } = await import('/__editor/mode-shell.js');
              const { mountModMode } = await import('/__editor/mod-mode-view.js');
              const { installModActionKeyboard } = await import('/__editor/mod-actions.js');
              const modeShell = new ModeShell();
              modeShell.switchMode('MOD');
              const view = mountModMode({
                host: document.getElementById('mod-mode-root'),
                modeShell,
                io: { readFile: async () => { throw new Error('not under test root'); } },
              });
              window.__feedback = [];
              window.addEventListener('phoenix-action-feedback', (event) => {
                window.__feedback.push(event.detail);
              });
              installModActionKeyboard({ target: document, modeShell, modActions: view });
              window.__modView = view;
              window.__editorReady = true;
              } catch (error) {
                window.__editorBootError = String(error && (error.stack || error.message || error));
                document.body.dataset.bootError = window.__editorBootError;
              }
            </script>
          </body>
        </html>`,
    });
  });
}

async function openEditor(context, page) {
  await routeCheckoutModules(context);
  await page.goto('/__editor-mod-import.html');
  await page.waitForFunction(() => window.__editorReady === true || !!window.__editorBootError);
  const bootError = await page.evaluate(() => window.__editorBootError || null);
  expect(bootError).toBeNull();
}

test('MOD import keyboard action applies a valid archive and states the M6 boundary', async ({
  context,
  page,
}) => {
  await openEditor(context, page);

  const importButton = page.getByRole('button', { name: /Import a ZIP mod pack/i });
  await importButton.focus();

  // A real committed valid ZIP reaches the existing editable workspace and the
  // shared local lifecycle, using only the semantic keyboard action.
  let chooserPromise = page.waitForEvent('filechooser');
  await page.keyboard.press('i');
  let chooser = await chooserPromise;
  await chooser.setFiles(modPackFixturePath('editor-round-trip'));

  await expect(page.locator('.mod-action-feedback')).toHaveAttribute('data-state', 'Applied');
  await expect(page.locator('.mod-import-success')).toBeVisible();
  await expect(page.locator('.mod-input-id')).toBeFocused();
  expect(await page.evaluate(() => window.__modView.getWorkspace().memberCount())).toBeGreaterThan(0);
  expect(await page.evaluate(() => window.__feedback.map((entry) => entry.state))).toEqual([
    'Pressed',
    'Pending',
    'Applied',
  ]);

  // The T2 surface says what remains M6 work and carries no hidden placeholder
  // controls for those deferred products.
  await expect(page.locator('.mod-scope-boundary')).toContainText(/M6/i);
  await expect(page.locator('[data-m6-inspector], [data-project-tooling]')).toHaveCount(0);
});

test('MOD import cancellation clears the provisional lifecycle on a fresh page', async ({
  context,
  page,
}) => {
  await openEditor(context, page);
  const importButton = page.getByRole('button', { name: /Import a ZIP mod pack/i });

  // Cancellation is not a refusal: it removes the provisional lifecycle and
  // restores focus to the launcher. This owns its page so Chromium's chooser
  // teardown cannot race a later chooser in another scenario.
  await importButton.focus();
  const chooserPromise = page.waitForEvent('filechooser');
  await page.keyboard.press('i');
  const chooser = await chooserPromise;
  await chooser.setFiles([]);
  await expect(importButton).toBeFocused();
  await expect(page.locator('.mod-action-feedback')).toBeEmpty();
  expect(await page.evaluate(() => window.__feedback.map((entry) => entry.state))).toEqual([
    'Pressed',
    'Pending',
    null,
  ]);
});

test('MOD import refuses a semantic-invalid archive and retains its source', async ({
  context,
  page,
}) => {
  await openEditor(context, page);

  // A real semantic-invalid ZIP is refused, but its exact source stays loaded
  // for repair and the non-colour alert receives focus.
  const chooserPromise = page.waitForEvent('filechooser');
  await page.keyboard.press('i');
  const chooser = await chooserPromise;
  await chooser.setFiles(modPackFixturePath('disallowed-path'));

  const findings = page.locator('.mod-import-findings[role="alert"]');
  await expect(findings).toBeFocused();
  await expect(findings).toContainText(/source bundle remains loaded and editable/i);
  await expect(findings).toContainText(/not a supported authored path/i);
  await expect(page.locator('.mod-member-row[data-path="assets/secret/keys.toml"]')).toHaveCount(1);
  await expect(page.locator('.mod-action-feedback')).toHaveAttribute('data-state', 'Refused');
  expect(await page.evaluate(() => window.__feedback.map((entry) => entry.state))).toEqual([
    'Pressed',
    'Pending',
    'Refused',
  ]);
});
