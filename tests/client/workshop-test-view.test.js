// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { TEST_GM_REFUSED_ACTIONS, mountWorkshopTestGm } from '../../gui/workshop-test-gm.js';
import { gmConsoleMarkup } from '../../scripts/gm-console-markup.mjs';

const SERVER = readFileSync('server.html', 'utf8');

describe('the disposable Test view', () => {
  afterEach(() => { vi.restoreAllMocks(); document.body.innerHTML = ''; });

  it('mounts the omniscient view without reading a single project asset', async () => {
    // The Test page's whole guarantee is that an uncaptured project file is
    // ABSENT, not fetched. The ordinary GM workspace reaches for the shipped
    // private-feedback manifest, the cue catalogue and every sample they name
    // the moment it mounts; the smoke isolation spec caught exactly that, so
    // this pins the mount itself rather than one panel's behaviour.
    // The map's custom element needs ResizeObserver and a 2D canvas, neither
    // of which jsdom has; it draws projections and fetches nothing, so it is
    // left out of the markup here rather than stubbed into half-existing.
    document.body.innerHTML = gmConsoleMarkup(SERVER)
      .replace(/<ph-navigation-map\b[\s\S]*?<\/ph-navigation-map>/g, '');
    const requested = [];
    vi.spyOn(window, 'fetch').mockImplementation(async input => {
      requested.push(String(input?.url ?? input));
      return new Response('', { status: 404 });
    });
    const gm = mountWorkshopTestGm({ win: window });
    expect(gm).not.toBeNull();
    await window.__privateAudio.ready;
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(requested.filter(url => /\/?assets\/(audio|sounds)\//.test(url))).toEqual([]);
    // No manifest and a silent output, reported as such rather than pretending.
    expect(window.__privateAudio.state().status).toBe('unavailable');
    // And the page says so where it mounts, so a reader sees why.
    expect(readFileSync('gui/workshop-test-gm.js', 'utf8')).toContain('isolated: true');
    gm.dispose();
  });

  it('tests captured role presets and typed widgets as local presentation that resets with the disposable page', () => {
    const markup = () => gmConsoleMarkup(SERVER)
      .replace(/<ph-navigation-map\b[\s\S]*?<\/ph-navigation-map>/g, '');
    const payload = JSON.stringify([{ id: 'draft-gm', label: 'draft.role',
      panels: ['gm-map-panel'], quick_actions: [], contacts: [], widget: [
        { id: 'brief', type: 'note', label: 'draft.brief', text: 'draft.note' },
        { id: 'controls', type: 'actions', label: 'draft.controls', actions: ['gm-session-pause'] },
      ] }]);
    document.body.innerHTML = markup();
    const first = mountWorkshopTestGm({ win: window });
    const refusePause = vi.fn(window.__hostSetSessionPaused);
    window.__hostSetSessionPaused = refusePause;
    window.__hostLocalGm = () => ({ id: 'test-only', name: 'Test only', connected: true });
    first.channel('gm_session', JSON.stringify({ phase: 'InProgress', paused: false, results: [] }));
    first.setRolePresets(payload);
    const select = document.getElementById('gm-role-preset-select');
    expect(document.querySelector('label[for="gm-role-preset-select"]')).not.toBeNull();
    expect([...select.options].map(option => option.value)).toEqual(['all', 'draft-gm']);
    select.value = 'draft-gm'; select.dispatchEvent(new Event('change'));
    expect(first.rolePresetState()).toMatchObject({ desiredId: 'draft-gm', effectivePresetId: 'draft-gm' });
    const card = document.querySelector('[data-widget-id="brief"]');
    expect(card.querySelector('[data-note-id="draft.note"]')).not.toBeNull();
    expect(card.getAttribute('aria-labelledby')).toBe(card.querySelector('h3').id);
    // The authored actions card presses the shipped session control. Even if
    // a Test page is hand-poked with an apparent operator, that ordinary path
    // reaches the explicit refusal and cannot become a silent no-op.
    const pause = document.querySelector('[data-widget-action="gm-session-pause"]');
    expect(pause.disabled).toBe(false);
    pause.click();
    expect(refusePause).toHaveBeenCalledWith(true, expect.any(String));
    expect(() => window.__hostGmCheckpointCreate('forbidden')).toThrow();
    // A captured list going stale falls back through the ordinary controller,
    // retaining only this disposable page's local choice for a reappearance.
    first.setRolePresets('[]');
    expect(first.rolePresetState()).toMatchObject({ desiredId: 'draft-gm', effectivePresetId: 'all' });
    first.setRolePresets(payload);
    expect(first.rolePresetState().effectivePresetId).toBe('draft-gm');
    first.dispose();

    // Restart is a fresh iframe and therefore a fresh local selection. No
    // reconnect identity or operator profile crosses the Test boundary.
    document.body.innerHTML = markup();
    const restarted = mountWorkshopTestGm({ win: window });
    restarted.setRolePresets(payload);
    expect(restarted.rolePresetState()).toMatchObject({ desiredId: null, effectivePresetId: 'all' });
    expect(document.getElementById('gm-widgets').hidden).toBe(true);
    restarted.dispose();
  });

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
    expect(TEST_GM_REFUSED_ACTIONS).toEqual(expect.arrayContaining([
      '__hostSetSessionPaused', '__hostGmCheckpointCreate',
    ]));
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
