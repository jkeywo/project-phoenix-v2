// @vitest-environment jsdom
//
// Issue #951 — the scenario-list rebuild must not delete the static
// "Upload mod pack" control.
//
// renderScenarioLockState() lives inline in server.html (classic script, closed
// over host state), so there is no importable module to call. Instead this test
// works from the real file: it parses server.html's actual #world-list markup
// and applies the *actual* cleanup selector the render function uses, read out
// of the source. That covers both halves of the regression — re-adding a
// lifecycle-matched class to the button, or reverting the selector back to
// `.world-btn` — without duplicating the render function's logic here.
//
// (A Playwright smoke assertion would need a built dist + WASM to reach the
// catalog stage at all; this runs in milliseconds and fails on the same edits.)

import { describe, it, expect, beforeAll } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const SERVER_HTML = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  '../../server.html',
);
const SRC = fs.readFileSync(SERVER_HTML, 'utf-8');

/** The lifecycle selector renderScenarioLockState() cleans up with. */
function scenarioEntrySelector() {
  const m = /const SCENARIO_ENTRY_SELECTOR = '([^']+)';/.exec(SRC);
  if (!m) throw new Error('SCENARIO_ENTRY_SELECTOR not found in server.html');
  return m[1];
}

/** The class renderScenarioLockState() stamps on the buttons it owns. */
function scenarioEntryClass() {
  const m = /const SCENARIO_ENTRY_CLASS = '([^']+)';/.exec(SRC);
  if (!m) throw new Error('SCENARIO_ENTRY_CLASS not found in server.html');
  return m[1];
}

/** A fresh copy of server.html's static #world-list subtree. */
function freshWorldList() {
  const doc = new DOMParser().parseFromString(SRC, 'text/html');
  const worldList = doc.getElementById('world-list');
  if (!worldList) throw new Error('#world-list not found in server.html');
  return { doc, worldList };
}

/** The rebuild's cleanup step, verbatim (server.html clearScenarioEntries). */
function clearScenarioEntries(worldList) {
  worldList.querySelectorAll(scenarioEntrySelector()).forEach((el) => el.remove());
}

describe('server.html scenario-list rebuild (issue #951)', () => {
  let SELECTOR;
  let ENTRY_CLASS;

  beforeAll(() => {
    SELECTOR = scenarioEntrySelector();
    ENTRY_CLASS = scenarioEntryClass();
  });

  it('places #mod-pack-btn inside the rebuilt container (the test is load-bearing)', () => {
    const { worldList } = freshWorldList();
    // If the button ever moves out of #world-list this test stops proving
    // anything, so assert the precondition the bug depended on.
    expect(worldList.querySelector('#mod-pack-btn')).not.toBeNull();
  });

  it('keeps #mod-pack-btn after the scenario stage rebuilds the list', () => {
    const { doc, worldList } = freshWorldList();

    // Two renders back-to-back: the catalog build renders once, then a mod-pack
    // upload calls refreshMergedCatalog() which renders again.
    for (let pass = 0; pass < 2; pass += 1) {
      clearScenarioEntries(worldList);
      for (const id of ['default', 'combat_test']) {
        const btn = doc.createElement('button');
        btn.className = `world-btn ${ENTRY_CLASS}`;
        btn.dataset.scenarioId = id;
        worldList.insertBefore(btn, worldList.querySelector('#mod-pack-upload'));
      }
      expect(worldList.querySelector('#mod-pack-btn')).not.toBeNull();
    }

    // Scenario buttons rendered, upload control still there and still clickable.
    expect(worldList.querySelectorAll('.world-btn[data-scenario-id]')).toHaveLength(2);
    const btn = worldList.querySelector('#mod-pack-btn');
    expect(btn).not.toBeNull();
    expect(btn.disabled).toBe(false);
    // Its siblings in the upload block survive too (they always did).
    expect(worldList.querySelector('#mod-pack-file')).not.toBeNull();
    expect(worldList.querySelector('#mod-pack-status')).not.toBeNull();
  });

  it('still removes the entries the rebuild owns', () => {
    const { doc, worldList } = freshWorldList();

    const stale = doc.createElement('button');
    stale.className = `world-btn ${ENTRY_CLASS}`;
    worldList.appendChild(stale);
    const picker = doc.createElement('ph-ship-picker');
    worldList.appendChild(picker);

    clearScenarioEntries(worldList);

    expect(worldList.contains(stale)).toBe(false);
    expect(worldList.querySelector('ph-ship-picker')).toBeNull();
    // #scenario-loading is the placeholder the rebuild replaces — matched by id
    // on purpose, unlike the upload control.
    expect(worldList.querySelector('#scenario-loading')).toBeNull();
    // Static furniture is untouched.
    expect(worldList.querySelector('#world-list-label')).not.toBeNull();
    expect(worldList.querySelector('#mod-pack-btn')).not.toBeNull();
  });

  it('keeps .world-btn as a styling-only hook on the upload button', () => {
    const { worldList } = freshWorldList();
    const btn = worldList.querySelector('#mod-pack-btn');
    // Appearance preserved: it still gets the shared button styling…
    expect(btn.classList.contains('world-btn')).toBe(true);
    // …but must not wear the rebuild's lifecycle class, nor match its selector.
    expect(btn.classList.contains(ENTRY_CLASS)).toBe(false);
    expect(btn.matches(SELECTOR)).toBe(false);
  });

  it('never cleans up #world-list by the shared .world-btn styling class', () => {
    // The original bug in one line: any querySelectorAll whose selector names
    // .world-btn sweeps up every static control styled as a button.
    expect(SRC).not.toMatch(/querySelectorAll\(\s*['"][^'"]*\.world-btn/);
  });

  it('stamps SCENARIO_ENTRY_CLASS on every scenario button it creates', () => {
    // The mirror-image bug: if the creation site stops applying the
    // lifecycle class, scenario buttons never match SCENARIO_ENTRY_SELECTOR,
    // so clearScenarioEntries() can no longer find them — they become
    // immortal and pile up in #world-list on every rebuild.
    expect(SRC).toMatch(/btn\.className = 'world-btn ' \+ SCENARIO_ENTRY_CLASS;/);
  });
});
