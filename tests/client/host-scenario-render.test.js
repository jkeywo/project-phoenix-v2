// @vitest-environment jsdom
//
// gui/host-scenario-render.js — the scenario picker's shared renderer
// (issue #1328).
//
// The claim under test is the one that makes "one picker" true rather than
// aspirational: TWO surfaces call this function — server.html's
// `renderScenarioLockState()` and the native host's lobby document
// (src/native_host/host_lobby/host_lobby_link.js) — over the SAME view model
// from gui/host-scenarios.js. So the cases here are driven through
// `scenarioCatalogView` wherever the stage is the point, rather than through a
// hand-written view model that could drift from what either caller passes.
//
// The markup is server.html's own `#scenario-panel` subtree, read off disk, for
// the same reason tests/client/scenario-list-lifecycle.test.js reads it: the
// renderer writes into element ids, and ids asserted against a stub prove
// nothing about the document either surface actually shows.

import { describe, it, expect, beforeEach } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  renderHostScenarios,
  SCENARIO_ENTRY_CLASS,
  SCENARIO_ENTRY_SELECTOR,
} from '../../gui/host-scenario-render.js';
import { scenarioCatalogView } from '../../gui/host-scenarios.js';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const SRC = fs.readFileSync(path.join(HERE, '../../server.html'), 'utf-8');

/** The two catalog entries every case below picks from. */
const CATALOG = [
  {
    id: 'combat_test',
    world: 'assets/worlds/combat_test.toml',
    label: 'world.combat_test.title',
    ships: [
      { template_path: 'assets/entities/alliance_destroyer.toml', label: 'Destroyer' },
      { template_path: 'assets/entities/alliance_cruiser.toml', label: 'Cruiser' },
    ],
  },
  {
    id: 'patrol',
    world: 'assets/worlds/patrol.toml',
    label: 'world.patrol.title',
    ships: [{ template_path: 'assets/entities/alliance_cruiser.toml' }],
  },
];

/** server.html's real scenario panel, in a fresh document. */
function panelDoc() {
  const parsed = new DOMParser().parseFromString(SRC, 'text/html');
  const panel = parsed.getElementById('scenario-panel');
  if (!panel) throw new Error('#scenario-panel not found in server.html');
  const doc = document.implementation.createHTMLDocument('');
  doc.body.appendChild(doc.importNode(panel, true));
  return doc;
}

/** A recording hook set — what each surface supplies in its own way. */
function hooks(extra) {
  const calls = { scenario: [], ship: [], auto: [] };
  return [
    {
      tData: (v) => (v == null ? '' : String(v)),
      selectScenario: (id) => calls.scenario.push(id),
      selectShip: (p) => calls.ship.push(p),
      autoSelectShip: (p) => calls.auto.push(p),
      shipStillNeeded: () => true,
      ...(extra || {}),
    },
    calls,
  ];
}

/** The string resolver both surfaces pass in; ids are enough to assert on. */
const t = (id) => `«${id}»`;

/**
 * Let the renderer's dynamic `import('./components/ph-ship-picker.js')` land.
 *
 * A module graph resolves over several microtask turns the first time it is
 * pulled in (the component imports the string table, the roving-tabindex helper
 * and `PhElement`), so one `setTimeout(0)` is not reliably enough. A handful of
 * turns is, and costs nothing once the module is cached.
 */
async function settle() {
  for (let i = 0; i < 10; i += 1) await new Promise((r) => setTimeout(r, 0));
}

let doc;
beforeEach(() => {
  doc = panelDoc();
});

describe('the scenario stage', () => {
  it('renders one button per catalog entry, above the static footer', () => {
    const vm = scenarioCatalogView(CATALOG, null, false);
    const [h, calls] = hooks();
    renderHostScenarios(doc, vm, t, h);

    const buttons = doc.querySelectorAll('.world-btn[data-scenario-id]');
    expect(Array.from(buttons).map((b) => b.dataset.scenarioId)).toEqual([
      'combat_test',
      'patrol',
    ]);
    // The world path rides along on the button, as it always has.
    expect(buttons[0].dataset.path).toBe('assets/worlds/combat_test.toml');
    // The column heading is the view model's id, resolved by the caller's `t`.
    expect(doc.getElementById('world-list-label').textContent).toBe('«server.select_world»');
    expect(calls.scenario).toEqual([]);
  });

  it('routes a click to the caller`s arbiter and nowhere else', () => {
    // The renderer has no arbiter and no transport: server.html reaches its own
    // page arbiter, the native surface reaches the host process. Both arrive as
    // this one hook.
    const [h, calls] = hooks();
    renderHostScenarios(doc, scenarioCatalogView(CATALOG, null, false), t, h);
    doc.querySelector('[data-scenario-id="patrol"]').click();
    expect(calls.scenario).toEqual(['patrol']);
    expect(calls.ship).toEqual([]);
  });

  it('falls back to the scenario id when the label resolves to nothing', () => {
    const [h] = hooks({ tData: () => '' });
    renderHostScenarios(doc, scenarioCatalogView(CATALOG, null, false), t, h);
    expect(doc.querySelector('[data-scenario-id="combat_test"]').textContent).toBe('combat_test');
  });

  it('says so when the manifest publishes nothing, rather than showing an empty column', () => {
    const [h] = hooks();
    renderHostScenarios(doc, scenarioCatalogView([], null, false), t, h);
    const placeholder = doc.getElementById('scenario-loading');
    expect(placeholder).not.toBeNull();
    expect(placeholder.textContent).toBe('«server.no_scenarios»');
  });
});

describe('the hull stage', () => {
  it('auto-resolves a scenario curated down to one hull without rendering a picker', () => {
    // Issue #917, count-based and keyed on no hull name. `autoSelectShip` is a
    // hook of its own because server.html answers it by calling its arbiter
    // DIRECTLY while a click goes through its action map.
    const vm = scenarioCatalogView(CATALOG, { scenario_id: 'patrol' }, false);
    expect(vm.stage).toBe('ship-auto');
    const [h, calls] = hooks();
    renderHostScenarios(doc, vm, t, h);
    expect(calls.auto).toEqual(['assets/entities/alliance_cruiser.toml']);
    expect(calls.ship).toEqual([]);
    // Nothing is built and nothing is torn down: the caller's arbiter answers,
    // and the answer re-renders. server.html's static #scenario-loading
    // placeholder is therefore still standing, exactly as it was.
    expect(doc.querySelector(`.${SCENARIO_ENTRY_CLASS}`)).toBeNull();
    expect(doc.querySelector('ph-ship-picker')).toBeNull();
  });

  it('falls back to selectShip when a caller supplies no auto hook', () => {
    const vm = scenarioCatalogView(CATALOG, { scenario_id: 'patrol' }, false);
    const [h, calls] = hooks({ autoSelectShip: undefined });
    renderHostScenarios(doc, vm, t, h);
    expect(calls.ship).toEqual(['assets/entities/alliance_cruiser.toml']);
  });

  it('mounts ph-ship-picker for a scenario with a choice, and carries the pick out', async () => {
    const vm = scenarioCatalogView(CATALOG, { scenario_id: 'combat_test' }, false);
    expect(vm.stage).toBe('ship-picker');
    const [h, calls] = hooks();
    renderHostScenarios(doc, vm, t, h);
    expect(doc.getElementById('world-list-label').textContent).toBe('«server.select_ship»');

    // The component load is a dynamic import; let it settle.
    await settle();
    const picker = doc.querySelector('ph-ship-picker');
    expect(picker).not.toBeNull();
    picker.dispatchEvent(
      new CustomEvent('ship-selected', {
        detail: { template_path: 'assets/entities/alliance_cruiser.toml' },
      }),
    );
    expect(calls.ship).toEqual(['assets/entities/alliance_cruiser.toml']);
  });

  it('does not mount a picker for a hull somebody else locked while it loaded', async () => {
    // First-valid-wins is not this renderer's rule, and the component import is
    // async: a phone can lock the hull between the render and the module
    // arriving. The caller re-checks its own authoritative state.
    const vm = scenarioCatalogView(CATALOG, { scenario_id: 'combat_test' }, false);
    const [h] = hooks({ shipStillNeeded: () => false });
    renderHostScenarios(doc, vm, t, h);
    await settle();
    expect(doc.querySelector('ph-ship-picker')).toBeNull();
  });
});

describe('the locked stage', () => {
  it('clears only what it owns, leaving the static tooling alone', () => {
    const [h] = hooks();
    renderHostScenarios(doc, scenarioCatalogView(CATALOG, null, false), t, h);
    expect(doc.querySelectorAll(`.${SCENARIO_ENTRY_CLASS}`).length).toBe(2);

    renderHostScenarios(doc, scenarioCatalogView(CATALOG, null, true), t, h);
    expect(doc.querySelector(SCENARIO_ENTRY_SELECTOR)).toBeNull();
    expect(doc.getElementById('mod-pack-btn')).not.toBeNull();
    expect(doc.getElementById('world-list-label')).not.toBeNull();
  });
});

describe('the panel`s own visibility', () => {
  it('is untouched unless a caller asks to own it (server.html`s behaviour)', () => {
    // On the host PAGE the panel is shown and hidden by page lifecycle —
    // driveWorldLoad() hides it, the return to lobby shows it again — and a
    // render must not start second-guessing that.
    const [h] = hooks();
    const before = doc.getElementById('scenario-panel').getAttribute('style');
    renderHostScenarios(doc, scenarioCatalogView(CATALOG, null, true), t, h);
    expect(doc.getElementById('scenario-panel').getAttribute('style')).toBe(before);
  });

  it('follows the stage for the surface that has no lifecycle of its own', () => {
    // The native viewscreen (issue #1328). Its document has no driveWorldLoad
    // and no return-to-lobby handler: the payload is the whole of what it
    // knows, so the renderer owns the panel on that surface.
    const [h] = hooks();
    const panel = doc.getElementById('scenario-panel');
    renderHostScenarios(doc, scenarioCatalogView(CATALOG, null, false), t, h, {
      ownPanelVisibility: true,
    });
    expect(panel.style.display).toBe('');

    renderHostScenarios(doc, scenarioCatalogView(CATALOG, null, true), t, h, {
      ownPanelVisibility: true,
    });
    expect(panel.style.display).toBe('none');
  });
});

describe('a document that carries no picker', () => {
  it('renders nothing rather than throwing', () => {
    // The mirror of renderHostLobby's element guards: one renderer serves two
    // documents, and a surface that carries the lobby without the picker (a
    // `--world` native host, before #1328 every native host) must not fault the
    // push it arrived on.
    const bare = document.implementation.createHTMLDocument('');
    const [h] = hooks();
    expect(() =>
      renderHostScenarios(bare, scenarioCatalogView(CATALOG, null, false), t, h),
    ).not.toThrow();
  });
});
