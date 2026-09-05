// @vitest-environment jsdom
/**
 * tests/client/console-core-tabs.test.js — the overlay-tab seam every console
 * gets from `initConsole` (issue #1373, PRD #1371).
 *
 * A console declares its overlay panels, the shell's Station Bar renders them
 * as tabs and selects one, and the console reports what its tab should be
 * carrying. This drives all three directions through the public seam — a real
 * document, a stand-in parent window, and `window.__updateConsole` — rather
 * than reaching into `initConsole`'s internals.
 *
 * It lives beside `console-core.test.js` rather than inside it because that
 * suite runs in bare Node with a hand-built `global.window`, and this one needs
 * a document to scan. `console-core-semantic-actions.test.js` split off for the
 * same reason.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { initConsole } from '../../gui/console-core.js';

/** Every message the console posted to its (stand-in) shell. */
let posted;
let disposers;

/**
 * Put a console document in the page and pretend it is inside an iframe, which
 * is the one context the tab seam posts to. Returns the runtime `initConsole`
 * hands back so the test can dispose it.
 */
function mountConsole(bodyHtml, opts = {}) {
  document.body.innerHTML = bodyHtml;
  const runtime = initConsole(Object.assign({ name: 'tactical', render: () => {} }, opts));
  disposers.push(runtime.disposeSemanticActions);
  return runtime;
}

const TACTICAL_BODY = `
  <div class="overlay-panel" id="security-overlay"
       data-tab-code="console.tactical.security.code"
       data-tab-name="console.tactical.security">
    <button class="overlay-toggle" data-overlay="security-overlay" data-active="false"></button>
  </div>
  <div class="overlay-panel" id="intel-overlay"
       data-tab-code="console.tactical.intel.code"
       data-tab-name="console.tactical.intel"></div>
`;

/** The last `console_tabs` message, or undefined. */
const lastTabs = () => posted.filter(m => m.type === 'console_tabs').at(-1);
/** The last `console_hull` message, or undefined. */
const lastHull = () => posted.filter(m => m.type === 'console_hull').at(-1);

beforeEach(() => {
  posted = [];
  disposers = [];
  // A stand-in shell. jsdom makes `window.parent` the window itself, which is
  // exactly the "not in an iframe" case the seam declines to post to.
  Object.defineProperty(window, 'parent', {
    configurable: true,
    value: { postMessage: (message) => posted.push(message) },
  });
});

afterEach(() => {
  for (const dispose of disposers) dispose();
  delete window.__setConsoleOverlay;
  delete window.__setConsoleTabBadge;
  delete window.__updateConsole;
  document.body.innerHTML = '';
  Reflect.deleteProperty(window, 'parent');
});

describe('a console declares its overlay panels', () => {
  it('posts one tab per panel that authored a tab code, in document order', () => {
    mountConsole(TACTICAL_BODY);
    expect(lastTabs()).toEqual({
      type: 'console_tabs',
      console: 'tactical',
      open: null,
      tabs: [
        {
          id: 'security-overlay',
          code: 'console.tactical.security.code',
          name: 'console.tactical.security',
          badge: 0,
        },
        {
          id: 'intel-overlay',
          code: 'console.tactical.intel.code',
          name: 'console.tactical.intel',
          badge: 0,
        },
      ],
    });
  });

  it('posts string ids, not display text — the shell resolves them', () => {
    mountConsole(TACTICAL_BODY);
    for (const tab of lastTabs().tabs) {
      expect(tab.code).toMatch(/^[a-z][a-z0-9_.]+$/);
      expect(tab.name).toMatch(/^[a-z][a-z0-9_.]+$/);
    }
  });

  it('leaves out a panel that declared no tab code — that is how one opts out', () => {
    mountConsole(`
      <div class="overlay-panel" id="intel-overlay" data-tab-code="console.tactical.intel.code"></div>
      <div class="overlay-panel" id="scratch-overlay"></div>
    `);
    expect(lastTabs().tabs.map(t => t.id)).toEqual(['intel-overlay']);
  });

  it('falls back to the code when a panel declared no name', () => {
    mountConsole('<div class="overlay-panel" id="intel-overlay" data-tab-code="INTL"></div>');
    expect(lastTabs().tabs[0]).toMatchObject({ code: 'INTL', name: 'INTL' });
  });

  it('a console with no overlays still declares — an empty list, not silence', () => {
    mountConsole('<div id="not-an-overlay"></div>', { name: 'helm' });
    expect(lastTabs())
      .toEqual({ type: 'console_tabs', console: 'helm', open: null, tabs: [] });
  });

  it('posts nothing at all when there is no shell above it', () => {
    // server.html, the wry host and separate-tab mode have no Station Bar.
    Object.defineProperty(window, 'parent', { configurable: true, value: window });
    mountConsole(TACTICAL_BODY);
    expect(posted).toEqual([]);
  });
});

describe('the shell selects an overlay', () => {
  it('opens exactly the named panel and closes the others', () => {
    mountConsole(TACTICAL_BODY);
    expect(window.__setConsoleOverlay('intel-overlay')).toBe('intel-overlay');
    expect(document.getElementById('intel-overlay').classList.contains('open')).toBe(true);
    window.__setConsoleOverlay('security-overlay');
    expect(document.getElementById('intel-overlay').classList.contains('open')).toBe(false);
    expect(document.getElementById('security-overlay').classList.contains('open')).toBe(true);
  });

  it('closes everything on a null selection', () => {
    mountConsole(TACTICAL_BODY);
    window.__setConsoleOverlay('intel-overlay');
    expect(window.__setConsoleOverlay(null)).toBeNull();
    expect(document.querySelectorAll('.overlay-panel.open')).toHaveLength(0);
  });

  it('re-renders on the tap, so a badge that clears on being read clears now', () => {
    const render = vi.fn();
    mountConsole(TACTICAL_BODY, { render });
    window.__updateConsole('tactical', JSON.stringify({ dossiers: [] }));
    expect(render).toHaveBeenCalledTimes(1);
    window.__setConsoleOverlay('intel-overlay');
    expect(render).toHaveBeenCalledTimes(2);
    // Same state object, so a render that reads the DOM sees the open panel.
    expect(render.mock.calls[1][0]).toEqual(render.mock.calls[0][0]);
  });

  it('reports which panel is open, so the bar can settle on it', () => {
    mountConsole(TACTICAL_BODY);
    expect(lastTabs().open).toBeNull();
    window.__setConsoleOverlay('intel-overlay');
    expect(lastTabs().open).toBe('intel-overlay');
    window.__setConsoleOverlay(null);
    expect(lastTabs().open).toBeNull();
  });

  it('re-declares when a panel is closed from INSIDE the console', () => {
    // The panel's own Back button, or a toggle the console still draws: the
    // document is the truth, so the console says so rather than leaving the
    // bar with a tab lit over nothing.
    mountConsole(TACTICAL_BODY);
    window.__setConsoleOverlay('intel-overlay');
    document.getElementById('intel-overlay').classList.remove('open');
    window.__updateConsole('tactical', JSON.stringify({}));
    expect(lastTabs().open).toBeNull();
  });

  it('does not re-declare on a push that changed nothing', () => {
    mountConsole(TACTICAL_BODY);
    window.__setConsoleOverlay('intel-overlay');
    const before = posted.filter(m => m.type === 'console_tabs').length;
    window.__updateConsole('tactical', JSON.stringify({}));
    window.__updateConsole('tactical', JSON.stringify({}));
    expect(posted.filter(m => m.type === 'console_tabs')).toHaveLength(before);
  });

  it('does not render before any state has arrived', () => {
    const render = vi.fn();
    mountConsole(TACTICAL_BODY, { render });
    window.__setConsoleOverlay('intel-overlay');
    expect(render).not.toHaveBeenCalled();
  });
});

describe('the console reports its badge', () => {
  it('re-posts the whole tab list with the new count', () => {
    mountConsole(TACTICAL_BODY);
    expect(window.__setConsoleTabBadge('intel-overlay', 3)).toBe(true);
    expect(lastTabs().tabs).toEqual([
      expect.objectContaining({ id: 'security-overlay', badge: 0 }),
      expect.objectContaining({ id: 'intel-overlay', badge: 3 }),
    ]);
  });

  it('posts nothing when the count has not moved', () => {
    mountConsole(TACTICAL_BODY);
    window.__setConsoleTabBadge('intel-overlay', 2);
    const before = posted.length;
    expect(window.__setConsoleTabBadge('intel-overlay', 2)).toBe(false);
    expect(posted).toHaveLength(before);
  });

  it('ignores a badge for a panel this console never declared', () => {
    mountConsole(TACTICAL_BODY);
    const before = posted.length;
    expect(window.__setConsoleTabBadge('nav-overlay', 4)).toBe(false);
    expect(posted).toHaveLength(before);
  });

  it('reads a negative or non-numeric count as nothing unread', () => {
    mountConsole(TACTICAL_BODY);
    window.__setConsoleTabBadge('intel-overlay', 5);
    window.__setConsoleTabBadge('intel-overlay', -2);
    expect(lastTabs().tabs.at(-1).badge).toBe(0);
    window.__setConsoleTabBadge('intel-overlay', 5);
    window.__setConsoleTabBadge('intel-overlay', 'lots');
    expect(lastTabs().tabs.at(-1).badge).toBe(0);
  });
});

describe('the own-Station system rows behind the bar damage popup', () => {
  it('posts the rows the payload carried', () => {
    mountConsole(TACTICAL_BODY);
    const entries = [{ system_id: 'phasers', display_name: 'Phasers', current: 4, max_hp: 10 }];
    window.__updateConsole('tactical', JSON.stringify({ own_hull: { entries } }));
    expect(lastHull()).toEqual({ type: 'console_hull', console: 'tactical', entries });
  });

  it('posts once for an unchanging hull, however many pushes arrive', () => {
    mountConsole(TACTICAL_BODY);
    const payload = JSON.stringify({
      own_hull: { entries: [{ system_id: 'phasers', current: 10, max_hp: 10 }] },
    });
    for (let i = 0; i < 5; i += 1) window.__updateConsole('tactical', payload);
    expect(posted.filter(m => m.type === 'console_hull')).toHaveLength(1);
  });

  it('posts again the moment a system takes damage', () => {
    mountConsole(TACTICAL_BODY);
    window.__updateConsole('tactical', JSON.stringify({
      own_hull: { entries: [{ system_id: 'phasers', current: 10, max_hp: 10 }] },
    }));
    window.__updateConsole('tactical', JSON.stringify({
      own_hull: { entries: [{ system_id: 'phasers', current: 6, max_hp: 10 }] },
    }));
    expect(posted.filter(m => m.type === 'console_hull')).toHaveLength(2);
    expect(lastHull().entries[0].current).toBe(6);
  });

  it('reports an empty list for a payload with no hull rows', () => {
    mountConsole(TACTICAL_BODY);
    window.__updateConsole('tactical', JSON.stringify({}));
    expect(lastHull()).toEqual({ type: 'console_hull', console: 'tactical', entries: [] });
  });
});
