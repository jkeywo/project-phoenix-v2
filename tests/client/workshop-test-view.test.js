// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { TEST_GM_REFUSED_ACTIONS } from '../../gui/workshop-test-gm.js';
import { gmConsoleMarkup } from '../../scripts/gm-console-markup.mjs';

const SERVER = readFileSync('server.html', 'utf8');

describe('the disposable Test view', () => {
  it('accepts only a view naming the GM desk or one ship', async () => {
    // The child's allow-list is the boundary: a control that is not exactly one
    // of these shapes never reaches the runtime at all.
    const { attachWorkshopTestChild } = await import('../../editor/workshop-test-child.js');
    expect(typeof attachWorkshopTestChild).toBe('function');
    const source = readFileSync('editor/workshop-test-child.js', 'utf8');
    expect(source).toContain("view.view === 'game-master'");
    expect(source).toContain("view.view === 'ship'");
    // A view carries no command, no target and no amount — nothing that could
    // be mistaken for an instruction to the simulation.
    expect(source).toContain("exact(view, ['view', 'entity'])");
  });

  it('draws the omniscient view from the REAL GM markup, not a copy', () => {
    const markup = gmConsoleMarkup(SERVER);
    // The same console the live GM uses, including panels added since.
    expect(markup).toContain('id="gm-console"');
    expect(markup).toContain('id="gm-map-panel"');
    expect(markup).toContain('id="gm-entity-fields-panel"');
    // And it carries no script: the Test page boots its own, deliberately.
    expect(markup).not.toMatch(/<script/i);
    // The page reserves the slot the build fills.
    expect(readFileSync('workshop-test.html', 'utf8')).toContain('<!--gm-console-->');
    expect(readFileSync('scripts/build-workshop.mjs', 'utf8')).toContain('gmConsoleMarkup');
  });

  it('installs every GM write as a refusal rather than leaving it undefined', () => {
    // A missing global would crash a panel on the first press and leave "can
    // this mutate the run?" answerable only by reading every panel. One list
    // answers it, and a Test that somehow acquired a route fails here.
    const live = readFileSync('gui/native-gm-workspace.js', 'utf8');
    const actions = [...live.matchAll(/(__host[A-Za-z]+):/g)].map(match => match[1]);
    expect(actions.length).toBeGreaterThan(10);
    for (const action of actions) {
      expect(TEST_GM_REFUSED_ACTIONS, `${action} must be refused in Test`).toContain(action);
    }
  });

  it('switches view through the runtime status, never the request', () => {
    // A refused switch must not leave the surface claiming a view the run is
    // not drawing, so the page follows what the runtime reports.
    const runtime = readFileSync('editor/workshop-test-runtime.js', 'utf8');
    expect(runtime).toContain('onView(status.view)');
    const boot = readFileSync('gui/workshop-test-boot.js', 'utf8');
    expect(boot).toContain("view?.view === 'game-master'");
  });

  it('refuses a ship the run does not have, in the runtime', () => {
    const browser = readFileSync('src/workshop/test_browser.rs', 'utf8');
    expect(browser.replace(/\s+/g, ' ')).toContain('status.ships.iter().any(');
  });

  it('switches with presentation-only state, so the run cannot notice', () => {
    const view = readFileSync('src/workshop/test_view.rs', 'utf8');
    // Both levers are already excluded from the authoritative fold.
    expect(view).toContain('StateClass::Presentation');
    expect(view).toContain('NativeGmPresentation');
    expect(view).toContain('LocalShip');
    // And neither writes anything the tick reads: no command but the two
    // markers, and no clock touched.
    expect(view).not.toContain('SimulationPaused');
    expect(view).not.toContain('TestClock');
  });
});
