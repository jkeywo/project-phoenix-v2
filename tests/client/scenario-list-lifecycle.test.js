// @vitest-environment jsdom
//
// Issue #951 — the scenario-list rebuild must not delete the static
// "Upload mod pack" control.
//
// The rebuild used to live inline in server.html (a classic script closed over
// host state), so this test worked from the file: it parsed the real
// #world-list markup and applied the cleanup selector read out of the source.
// Issue #1328 extracted every one of those writes into
// gui/host-scenario-render.js — the native host shows the same picker on its
// viewscreen — so the test now drives the REAL renderer against that same real
// markup. Both halves of the regression are still covered (re-adding a
// lifecycle-matched class to the static button, or reverting the selector back
// to `.world-btn`), and the creation site is now exercised rather than
// pattern-matched.
//
// (A Playwright smoke assertion would need a built dist + WASM to reach the
// catalog stage at all; this runs in milliseconds and fails on the same edits.)

import { describe, it, expect } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  renderHostScenarios,
  clearScenarioEntries,
  SCENARIO_ENTRY_CLASS,
  SCENARIO_ENTRY_SELECTOR,
} from '../../gui/host-scenario-render.js';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const SERVER_HTML = path.join(HERE, '../../server.html');
const SRC = fs.readFileSync(SERVER_HTML, 'utf-8');
const RENDERER = fs.readFileSync(path.join(HERE, '../../gui/host-scenario-render.js'), 'utf-8');

/** A fresh copy of server.html's static #world-list subtree. */
function freshWorldList() {
  const doc = new DOMParser().parseFromString(SRC, 'text/html');
  const worldList = doc.getElementById('world-list');
  if (!worldList) throw new Error('#world-list not found in server.html');
  return { doc, worldList };
}

/** The scenario stage's view model, as gui/host-scenarios.js returns it. */
function scenarioListVm(ids) {
  return {
    stage: 'scenario-list',
    labelId: 'server.select_world',
    entries: ids.map((id) => ({ scenarioId: id, world: `assets/worlds/${id}.toml`, label: id })),
  };
}

/** Render with no hooks that matter — this suite is about the list lifecycle. */
function render(doc, vm) {
  renderHostScenarios(doc, vm, (id) => id, { tData: (v) => String(v || '') });
}

describe('server.html scenario-list rebuild (issue #951)', () => {
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
      render(doc, scenarioListVm(['default', 'combat_test']));
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

  it('inserts its entries above the static footer, not below it', () => {
    // What `margin-top: auto` on #mod-pack-upload is for: a plain appendChild
    // stacks scenario buttons below the upload block's separator rule.
    const { doc, worldList } = freshWorldList();
    render(doc, scenarioListVm(['combat_test']));
    const children = Array.from(worldList.children);
    const entry = children.findIndex((el) => el.matches(SCENARIO_ENTRY_SELECTOR));
    const footer = children.findIndex((el) => el.id === 'mod-pack-upload');
    expect(entry).toBeGreaterThanOrEqual(0);
    expect(footer).toBeGreaterThan(entry);
  });

  it('still removes the entries the rebuild owns', () => {
    const { doc, worldList } = freshWorldList();

    const stale = doc.createElement('button');
    stale.className = `world-btn ${SCENARIO_ENTRY_CLASS}`;
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
    expect(btn.classList.contains(SCENARIO_ENTRY_CLASS)).toBe(false);
    expect(btn.matches(SCENARIO_ENTRY_SELECTOR)).toBe(false);
  });

  it('never cleans up #world-list by the shared .world-btn styling class', () => {
    // The original bug in one line: any querySelectorAll whose selector names
    // .world-btn sweeps up every static control styled as a button. Scanned in
    // both files now — the writes moved out of the page in #1328, and the page
    // must not grow a second sweep of its own either.
    expect(SRC).not.toMatch(/querySelectorAll\(\s*['"][^'"]*\.world-btn/);
    expect(RENDERER).not.toMatch(/querySelectorAll\(\s*['"][^'"]*\.world-btn/);
  });

  it('stamps SCENARIO_ENTRY_CLASS on every scenario button it creates', () => {
    // The mirror-image bug: if the creation site stops applying the lifecycle
    // class, scenario buttons never match SCENARIO_ENTRY_SELECTOR, so
    // clearScenarioEntries() can no longer find them — they become immortal and
    // pile up in #world-list on every rebuild. Asserted by rendering twice and
    // counting, which is the failure itself rather than a proxy for it.
    const { doc, worldList } = freshWorldList();
    render(doc, scenarioListVm(['default', 'combat_test']));
    render(doc, scenarioListVm(['default', 'combat_test']));
    expect(worldList.querySelectorAll('.world-btn[data-scenario-id]')).toHaveLength(2);
    for (const btn of worldList.querySelectorAll('.world-btn[data-scenario-id]')) {
      expect(btn.classList.contains(SCENARIO_ENTRY_CLASS)).toBe(true);
    }
  });
});
