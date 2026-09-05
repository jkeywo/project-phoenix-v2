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

import { describe, it, expect, beforeEach, afterEach } from 'vitest';
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

/**
 * server.html's real scenario panel, in a fresh document.
 *
 * WITHOUT the landing's hull column, deliberately: this is the shape the
 * native lobby document carries, and it drives the hull stage's fallback
 * branch. `stagedDoc()` below is the other surface.
 */
function panelDoc() {
  const parsed = new DOMParser().parseFromString(SRC, 'text/html');
  const panel = parsed.getElementById('scenario-panel');
  if (!panel) throw new Error('#scenario-panel not found in server.html');
  const doc = document.implementation.createHTMLDocument('');
  doc.body.appendChild(doc.importNode(panel, true));
  return doc;
}

/**
 * The same panel PLUS the landing's hull column (issue #1362).
 *
 * Read off server.html rather than hand-built, for the reason `panelDoc()` is:
 * this renderer finds the column by id, and an id asserted against a stub
 * proves nothing about the document the host actually shows.
 */
function stagedDoc() {
  const parsed = new DOMParser().parseFromString(SRC, 'text/html');
  const column = parsed.getElementById('landing-ship');
  if (!column) throw new Error('#landing-ship not found in server.html');
  const doc = panelDoc();
  doc.body.appendChild(doc.importNode(column, true));
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
 * Spin the event loop until `ready()` answers, or a real-time deadline passes.
 *
 * The renderer's `import('./components/ph-ship-picker.js')` resolves a whole
 * module graph the first time it is pulled in — the component imports the
 * string table, the roving-tabindex helper and `PhElement` — and how many turns
 * that takes is a function of how busy the machine is, not of the code. A fixed
 * count therefore passes on an idle box and fails inside a loaded
 * `vitest run` of every suite at once, which is a flake rather than a finding.
 * Waiting on the CONDITION with a generous ceiling is both faster in the normal
 * case and honest in the slow one.
 */
async function settleUntil(ready) {
  const deadline = Date.now() + 5000;
  while (!ready() && Date.now() < deadline) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

/**
 * Spin a fixed handful of turns, for asserting something did NOT happen.
 *
 * There is no condition to wait on in that case. It is only sound because the
 * mounting test above runs first and leaves the module graph cached, so the
 * import this is giving room to resolves immediately.
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
    // `.world-btn-name` and not the button's whole textContent: the row also
    // carries its hull count now (issue #1362), and the fallback is about the
    // NAME.
    expect(doc.querySelector('[data-scenario-id="combat_test"] .world-btn-name').textContent)
      .toBe('combat_test');
  });

  it('says on each row how many hulls it offers, and marks the one that will ask', () => {
    // Issue #1362. Two hulls is a choice and reads `.on`; one hull goes
    // straight through and does not.
    const [h] = hooks();
    renderHostScenarios(doc, scenarioCatalogView(CATALOG, null, false), t, h);
    const chip = (id) => doc.querySelector(`[data-scenario-id="${id}"] .world-btn-hulls`);
    expect(chip('combat_test').textContent).toBe('«server.hulls_offered.other»');
    expect(chip('combat_test').classList.contains('on')).toBe(true);
    expect(chip('patrol').textContent).toBe('«server.hulls_offered.one»');
    expect(chip('patrol').classList.contains('on')).toBe(false);
  });

  it('draws no count on a World that publishes no curated hull list', () => {
    // Empty means UNRESTRICTED, not none — a "0 hulls" chip would be the one
    // wrong reading. Nothing is drawn instead.
    const open = [{ id: 'open', world: 'assets/worlds/open.toml', label: 'Open' }];
    const [h] = hooks();
    renderHostScenarios(doc, scenarioCatalogView(open, null, false), t, h);
    expect(doc.querySelector('[data-scenario-id="open"] .world-btn-hulls')).toBeNull();
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
    await settleUntil(() => doc.querySelector('ph-ship-picker'));
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

  it('draws a Back control only where a surface supplied an arbiter for it', () => {
    // A control exists exactly when something behind it can answer it. The
    // native surface reaches a different arbiter and may supply no hook.
    const vm = scenarioCatalogView(CATALOG, { scenario_id: 'combat_test' }, false);
    const [bare] = hooks();
    renderHostScenarios(doc, vm, t, bare);
    expect(doc.querySelector('.scenario-back')).toBeNull();

    const back = [];
    const [h] = hooks({ backToWorlds: () => back.push(1) });
    renderHostScenarios(doc, vm, t, h);
    const btn = doc.querySelector('.scenario-back');
    expect(btn).not.toBeNull();
    btn.click();
    expect(back).toEqual([1]);
  });

  it('takes the Back control away again on every other stage', () => {
    const vm = scenarioCatalogView(CATALOG, { scenario_id: 'combat_test' }, false);
    const [h] = hooks({ backToWorlds: () => {} });
    renderHostScenarios(doc, vm, t, h);
    expect(doc.querySelector('.scenario-back')).not.toBeNull();
    renderHostScenarios(doc, scenarioCatalogView(CATALOG, null, false), t, h);
    expect(doc.querySelector('.scenario-back')).toBeNull();
  });
});

describe('the hull stage BESIDE the World list (issue #1362)', () => {
  // The staged layout, on a surface that carries `#ship-list`. Choosing a
  // World reveals the hulls in their own column and leaves the World rows
  // standing, so the operator can see the path they took and step back along
  // it. Which surface that is, is the DOCUMENT's answer — this renderer finds
  // the column by id rather than being told about it.
  let staged;
  beforeEach(() => { staged = stagedDoc(); });

  const shipStage = () => scenarioCatalogView(CATALOG, { scenario_id: 'combat_test' }, false);

  it('keeps the World rows in their own column, with the chosen one marked', async () => {
    const [h] = hooks();
    renderHostScenarios(staged, shipStage(), t, h);
    await settleUntil(() => staged.querySelector('ph-ship-picker'));

    const rows = staged.querySelectorAll('#world-list .world-btn[data-scenario-id]');
    expect(Array.from(rows).map((b) => b.dataset.scenarioId)).toEqual(['combat_test', 'patrol']);
    const chosen = staged.querySelector('[data-scenario-id="combat_test"]');
    expect(chosen.classList.contains('active')).toBe(true);
    expect(chosen.getAttribute('aria-current')).toBe('true');
    expect(staged.querySelector('[data-scenario-id="patrol"]').getAttribute('aria-current')).toBeNull();
  });

  it('mounts the hulls in the hull column and not in the World column', async () => {
    const [h] = hooks();
    renderHostScenarios(staged, shipStage(), t, h);
    await settleUntil(() => staged.querySelector('ph-ship-picker'));
    expect(staged.querySelector('#ship-list ph-ship-picker')).not.toBeNull();
    expect(staged.querySelector('#world-list ph-ship-picker')).toBeNull();
  });

  it('heads each column with its own label rather than renaming the World one', () => {
    // The failure this catches is the one the slice exists to undo: the world
    // column used to become the ship column, heading and all.
    const [h] = hooks();
    renderHostScenarios(staged, shipStage(), t, h);
    expect(staged.getElementById('world-list-label').textContent).toBe('«server.select_world»');
    expect(staged.getElementById('ship-list-label').textContent).toBe('«server.select_ship»');
  });

  it('puts Back in the hull column, one click from the World list', async () => {
    const back = [];
    const [h] = hooks({ backToWorlds: () => back.push(1) });
    renderHostScenarios(staged, shipStage(), t, h);
    await settleUntil(() => staged.querySelector('ph-ship-picker'));
    const btn = staged.querySelector('#ship-list .scenario-back');
    expect(btn).not.toBeNull();
    // Under the cards, as the design draws it — and that order does not depend
    // on how long the component's dynamic import took.
    const kids = Array.from(staged.getElementById('ship-list').children);
    expect(kids.map((el) => el.tagName.toLowerCase()))
      .toEqual(['ph-ship-picker', 'button']);
    btn.click();
    expect(back).toEqual([1]);
  });

  it('empties the hull column on every stage that is not the hull stage', async () => {
    const [h] = hooks({ backToWorlds: () => {} });
    renderHostScenarios(staged, shipStage(), t, h);
    await settleUntil(() => staged.querySelector('ph-ship-picker'));

    renderHostScenarios(staged, scenarioCatalogView(CATALOG, null, false), t, h);
    expect(staged.getElementById('ship-list').children).toHaveLength(0);

    renderHostScenarios(staged, shipStage(), t, h);
    await settleUntil(() => staged.querySelector('ph-ship-picker'));
    renderHostScenarios(staged, scenarioCatalogView(CATALOG, null, true), t, h);
    expect(staged.getElementById('ship-list').children).toHaveLength(0);
  });

  it('does not resurrect the stage a Back left, when the import lands afterwards', async () => {
    // The component load is async and the operator is faster than a module
    // graph: a Back between the render and the import arriving must not put
    // the cards (or the Back button) back into a column that has moved on.
    const [h] = hooks({ backToWorlds: () => {} });
    renderHostScenarios(staged, shipStage(), t, h);
    renderHostScenarios(staged, scenarioCatalogView(CATALOG, null, false), t, h);
    await settle();
    expect(staged.getElementById('ship-list').children).toHaveLength(0);
  });
});

describe('keyboard focus across a Back (issue #1362)', () => {
  // Back deletes the control the operator is standing on and rebuilds
  // `#world-list` from scratch, so without help focus falls to `<body>` and
  // the next Tab restarts at the top of the document — on the surface
  // gui/host-landing-render.js calls the one most likely to be driven entirely
  // by keys. It is worse than the forward click that has the same shape,
  // because Back is a step BACKWARDS: the operator's place is known rather
  // than gone.
  //
  // jsdom tracks `activeElement` only for a document with a browsing context,
  // so these drive the ambient document rather than `stagedDoc()` — which is
  // also the shape both real surfaces hand the renderer.
  afterEach(() => { document.body.innerHTML = ''; });

  /**
   * A catalog whose multi-hull World is NOT the first row, so "the row they
   * released" is distinguishable from "the top of the list".
   */
  const OFFSET_CATALOG = [
    { id: 'patrol', world: 'assets/worlds/patrol.toml', label: 'world.patrol.title', ships: [] },
    {
      id: 'combat_test',
      world: 'assets/worlds/combat_test.toml',
      label: 'world.combat_test.title',
      ships: [
        { template_path: 'assets/entities/alliance_destroyer.toml' },
        { template_path: 'assets/entities/alliance_cruiser.toml' },
      ],
    },
  ];

  function stagedInWindow() {
    const parsed = new DOMParser().parseFromString(SRC, 'text/html');
    document.body.innerHTML = '';
    document.body.appendChild(
      document.importNode(parsed.getElementById('scenario-panel'), true));
    document.body.appendChild(
      document.importNode(parsed.getElementById('landing-ship'), true));
    return document;
  }

  const shipStage = () =>
    scenarioCatalogView(OFFSET_CATALOG, { scenario_id: 'combat_test' }, false);
  const worldStage = () => scenarioCatalogView(OFFSET_CATALOG, null, false);

  it('carries the released World on the Back control itself', () => {
    // How the next render knows where to put the operator, without this module
    // remembering a previous view model for two documents at once.
    const [h] = hooks({ backToWorlds: () => {} });
    const doc = stagedInWindow();
    renderHostScenarios(doc, shipStage(), t, h);
    expect(doc.querySelector('.scenario-back').dataset.scenarioId).toBe('combat_test');
  });

  it('puts the operator back on the World row they released', () => {
    const [h] = hooks({ backToWorlds: () => {} });
    const doc = stagedInWindow();
    renderHostScenarios(doc, shipStage(), t, h);
    doc.querySelector('.scenario-back').focus();
    expect(doc.activeElement.classList.contains('scenario-back')).toBe(true);

    // What the click does: the caller releases the World and re-renders, which
    // is the render that deletes the button under the operator.
    renderHostScenarios(doc, worldStage(), t, h);

    expect(doc.activeElement.dataset.scenarioId).toBe('combat_test');
    // ...and it is the row this render CREATED, not a detached node that still
    // answers to `focus()`.
    expect(doc.getElementById('world-list').contains(doc.activeElement)).toBe(true);
  });

  it('does not pull focus back from wherever the operator has moved on to', () => {
    // The restore is conditioned on the OUTGOING focus having been Back's, so
    // a render that lands while the operator is somewhere else leaves them.
    const [h] = hooks({ backToWorlds: () => {} });
    const doc = stagedInWindow();
    renderHostScenarios(doc, shipStage(), t, h);
    const elsewhere = doc.getElementById('mod-pack-btn');
    elsewhere.focus();
    renderHostScenarios(doc, worldStage(), t, h);
    expect(doc.activeElement).toBe(elsewhere);
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
