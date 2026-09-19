import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import path from 'node:path';

// scripts/build-workshop.mjs copies `gui/` wholesale but names each editor
// module it ships ONE BY ONE. A module the Workshop pages import that is
// missing from that list is not a build error and not a unit-test failure: the
// built page's module graph simply fails to load and an empty <main> is served.
// Issue #1474 shipped exactly that (editor/workshop-definitions.js), and only a
// real build followed by a smoke run noticed. This walks the static import
// graph from every Workshop entry point and refuses the drift up front.
const ROOT = path.resolve(__dirname, '../..');
const ENTRY_POINTS = ['gui/workshop-boot.js', 'gui/workshop-test-boot.js', 'gui/workshop-preview-boot.js'];
const IMPORT = /(?:^|\n)\s*import(?:[^'"]*?from)?\s*['"]([^'"]+)['"]/g;

function shippedEditorModules() {
  const script = readFileSync(path.join(ROOT, 'scripts/build-workshop.mjs'), 'utf8');
  const list = /const editorModules = \[([\s\S]*?)\];/.exec(script);
  expect(list, 'scripts/build-workshop.mjs declares editorModules').toBeTruthy();
  return new Set([...list[1].matchAll(/'([^']+)'/g)].map(match => match[1]));
}

function reachableModules() {
  const seen = new Set();
  const queue = ENTRY_POINTS.map(entry => path.join(ROOT, entry));
  while (queue.length) {
    const file = queue.pop();
    const relative = path.relative(ROOT, file).replace(/\\/g, '/');
    if (seen.has(relative)) continue;
    seen.add(relative);
    const source = readFileSync(file, 'utf8');
    for (const match of source.matchAll(IMPORT)) {
      const specifier = match[1];
      // Bare specifiers (smol-toml) are vendored by the same build script and
      // are not this test's concern; only the repo's own relative modules are.
      if (!specifier.startsWith('.')) continue;
      const target = path.resolve(path.dirname(file), specifier);
      const targetRelative = path.relative(ROOT, target).replace(/\\/g, '/');
      if (/^(gui|editor)\//.test(targetRelative) && targetRelative.endsWith('.js')) queue.push(target);
    }
  }
  return seen;
}

describe('the Workshop build ships every editor module its pages import', () => {
  it('lists each reachable editor module in scripts/build-workshop.mjs', () => {
    const shipped = shippedEditorModules();
    const missing = [...reachableModules()]
      .filter(file => file.startsWith('editor/'))
      .map(file => file.slice('editor/'.length, -'.js'.length))
      .filter(name => !shipped.has(name))
      .sort();
    expect(missing, 'add these to editorModules in scripts/build-workshop.mjs').toEqual([]);
  });

  it('reaches the modules the pages are known to need, so the walk is not vacuous', () => {
    const reached = reachableModules();
    for (const file of ['gui/workshop-authoring.js', 'editor/workshop-document.js', 'editor/workshop-definitions.js',
      'editor/workshop-composition.js', 'editor/workshop-entity.js', 'editor/workshop-presets.js',
      'editor/workshop-test-runtime.js', 'editor/workshop-preview-runtime.js']) {
      expect(reached.has(file), file).toBe(true);
    }
  });
});
