// @vitest-environment jsdom
/**
 * tests/client/viewscreen-hud.test.js — the native Viewscreen's HUD overlay
 * (gui/viewscreen-hud.html, issue #422's overlay ported to the native path).
 *
 * Nothing else executes this page. `scripts/check-strings.mjs` reads it, but
 * statically: it checks that every `data-i18n` id has a CSV row and never runs
 * a line of it, so deleting the whole module island leaves that gate green with
 * the attributes inert — and the four status-slot values it writes at runtime
 * are handed to a `hud.setText(id, value)` helper the scanner cannot see
 * through at all. This suite is what stands behind both.
 *
 * **It drives the real page.** The markup, the classic prelude and the module
 * island are all read out of the .html file and run here in the order the
 * browser runs them, against the repository's own gui/ modules rather than
 * stubs: `applyToDom`, `t`, `localiseTree` and `localiseHostPayload` are the
 * shipped ones, and the payloads below are the shape `panes/ultralight.rs`
 * pushes through `window.__updateHud`.
 *
 * The three failure modes it exists to catch, all of which look identical from
 * outside — a Viewscreen that shows nothing:
 *
 *   1. the module island stops running (deleted, or an import that 404s in a
 *      trimmed --client-dir), so the localising renderer is never installed;
 *   2. the String Table does not load, so every id would render as ⟨…⟩;
 *   3. the host pushes before the deferred module has evaluated.
 */

import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import path from 'node:path';
import { existsSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

import { applyToDom, getTable, localiseTree, setTable, t } from '../../gui/strings.js';
import { localiseHostPayload } from '../../gui/host-channel.js';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const HTML = readFileSync(path.join(root, 'gui/viewscreen-hud.html'), 'utf8');

/**
 * The page split into the three pieces the browser treats separately.
 *
 * Parsed rather than regexed off the text: the file's own comments talk ABOUT
 * `<body>` and about the scripts, and a parser is the only reader that knows
 * the difference between a tag and a sentence mentioning one. `parseFromString`
 * never executes what it finds, so the scripts arrive here as source.
 */
const { MARKUP, CLASSIC, MODULE } = (() => {
  const doc = new DOMParser().parseFromString(HTML, 'text/html');
  const scripts = [...doc.querySelectorAll('script')];
  const classic = scripts.find((s) => !s.type);
  const island = scripts.find((s) => s.type === 'module');
  if (!classic || !island) throw new Error('viewscreen-hud.html no longer carries both scripts');
  for (const s of scripts) s.remove();
  return { MARKUP: doc.body.innerHTML, CLASSIC: classic.textContent, MODULE: island.textContent };
})();

/** The overlay's markup plus its classic prelude, as the parser leaves them. */
function mountPage() {
  document.body.innerHTML = MARKUP;
  // A classic <script> runs in global scope, not in a module's — `var hud` and
  // the IIFE's `window.__phoenixHud` both depend on that.
  // eslint-disable-next-line no-new-func
  new Function(CLASSIC)();
}

/**
 * The `./x.js` files the island imports, and the names it destructures.
 *
 * Anchored to an `import` STATEMENT rather than to any `'./x.js'` in the text:
 * the island's own comment names one of the specifiers while explaining where
 * the table comes from, and counting that would make this a check on prose.
 */
const SPECIFIERS = [...MODULE.matchAll(/^\s*import\s[^;]*'\.\/([A-Za-z0-9_.-]+\.js)'/gm)]
  .map((m) => m[1]);
const IMPORTED = [...MODULE.matchAll(/^\s*import\s*\{([^}]*)\}/gm)]
  .flatMap((m) => m[1].split(','))
  .map((name) => name.trim())
  .filter(Boolean)
  .sort();

/** What `runIsland` binds those names to, in declaration order. */
const BINDINGS = { applyToDom, getTable, localiseTree, t, localiseHostPayload };

/**
 * Run the island's body with its imports bound to the real gui/ modules.
 *
 * Evaluated rather than `import()`ed, and the difference matters: a dynamic
 * import of a rewritten source string is resolved by Node's own loader, which
 * hands it a SECOND copy of gui/strings.js with a table of its own. The island
 * then reads an empty table no matter what the test installed, `hasTable` is
 * false, and every assertion about localised text passes or fails for a reason
 * that has nothing to do with the page. Binding the test's own module instances
 * keeps one table, which is the whole subject here.
 *
 * The `import` lines are stripped, so what the specifiers point at is checked
 * separately below rather than by resolving them.
 */
function runIsland() {
  const body = MODULE.replace(/^\s*import[^;]*;/gm, '');
  // eslint-disable-next-line no-new-func
  new Function(...Object.keys(BINDINGS), body)(...Object.values(BINDINGS));
}

/** A HUD-state push, in the shape `codec::encode_hud_state` produces. */
function push(state) {
  window.__updateHud(JSON.stringify({
    heading: 90,
    hull_pct: 84,
    red_alert: false,
    condition: 'server.hud_nominal',
    game_over_message: null,
    ...state,
  }));
}

const text = (id) => document.getElementById(id)?.textContent;
const slotLabel = (id) => document.querySelector(`[data-i18n="${id}"]`)?.textContent;

/** The real table, restored after any test that takes it away. */
const REAL_TABLE = getTable();

beforeEach(() => {
  setTable(REAL_TABLE);
  document.body.innerHTML = '';
  delete window.__phoenixHud;
  delete window.__updateHud;
});

afterEach(() => {
  setTable(REAL_TABLE);
});

describe('the page localises itself from the String Table', () => {
  it('substitutes the static markup once the island has a table', () => {
    // Asserted against a table this test EDITS, because the four slot labels
    // are authored to match the English already in the markup: comparing them
    // to t() alone would pass just as happily if applyToDom never ran, which is
    // the shape of vacuous test this suite exists to replace.
    setTable(new Map(REAL_TABLE).set('server.hud_nav', 'NAVIGATION'));
    mountPage();
    expect(slotLabel('server.hud_nav')).toBe('Nav');

    runIsland();
    expect(slotLabel('server.hud_nav')).toBe('NAVIGATION');
    expect(slotLabel('server.hud_tac')).toBe(t('server.hud_tac'));
    expect(slotLabel('server.hud_eng')).toBe(t('server.hud_eng'));
    expect(slotLabel('server.hud_sys')).toBe(t('server.hud_sys'));
    expect(text('hud-designation')).toBe(t('server.hud_designation'));
    expect(document.body.textContent).not.toContain('⟨');
  });

  it('imports exactly the modules it is run against, and they are on disk', () => {
    // The one thing evaluating the island's body cannot check for itself. A
    // specifier that resolves to nothing is failure mode 1 in this file's
    // header — the island never evaluates and the prelude's fallback carries
    // the Viewscreen — and a name added to an import list would otherwise be
    // an undefined binding here rather than a failing expectation.
    expect(SPECIFIERS).toEqual(['strings-boot.js', 'strings.js', 'host-channel.js']);
    for (const file of SPECIFIERS) {
      expect(existsSync(path.join(root, 'gui', file)), `gui/${file}`).toBe(true);
    }
    expect(IMPORTED).toEqual(Object.keys(BINDINGS).sort());
  });

  it('draws the four runtime values through the ids server.html draws them through', () => {
    // The finding this pins: `'HDG ' + heading`, `'WEAPONS HOT'`, `'CLEAR'` and
    // `'HULL ' + pct + '%'` were hand-written English here — two of them
    // transcriptions of the very rows the page now resolves — while
    // server.html's copy of the same overlay rendered the same four slots
    // through t(). check-strings cannot see a literal handed to setText(), so
    // only this assertion stands between the two Viewscreens and drifting apart.
    mountPage();
    runIsland();
    push({ heading: 90, hull_pct: 84 });

    expect(text('v-nav')).toBe(t('server.hud_heading', { deg: '090' }));
    expect(text('v-tac')).toBe(t('server.hud_clear'));
    expect(text('v-eng')).toBe(t('server.hud_hull', { pct: 84 }));
    expect(text('v-sys')).toBe(t('server.hud_nominal'));
    expect(text('v-sys')).not.toBe('server.hud_nominal');
    expect(MODULE).toContain('server.hud_heading');
    expect(MODULE).toContain('server.hud_hull');
  });

  it('goes to red alert in the border, the vignette and the tactical slot', () => {
    mountPage();
    runIsland();
    push({ red_alert: true, condition: 'server.hud_alert' });

    expect(document.getElementById('hud-overlay').classList.contains('alert-on')).toBe(true);
    expect(text('v-tac')).toBe(t('server.hud_weapons_hot'));
    expect(text('v-sys')).toBe(t('server.hud_alert'));

    push({ red_alert: false });
    expect(document.getElementById('hud-overlay').classList.contains('alert-on')).toBe(false);
    expect(text('v-tac')).toBe(t('server.hud_clear'));
  });

  it('resolves a built-in ending and passes a scenario’s own prose through', () => {
    mountPage();
    runIsland();
    const overlay = () => document.getElementById('game-over-overlay').style.display;

    expect(overlay()).toBe('none');
    push({ game_over_message: 'server.game_over.ship_destroyed' });
    expect(text('game-over-message')).toBe(t('server.game_over.ship_destroyed'));
    expect(overlay()).toBe('flex');

    // Not an id — a mission's authored closing line. localiseTree substitutes
    // only what the table holds, so this must arrive verbatim.
    push({ game_over_message: 'The skyway holds. Go home.' });
    expect(text('game-over-message')).toBe('The skyway holds. Go home.');

    // Cleared on a new run rather than left over the live scene.
    push({ game_over_message: null });
    expect(overlay()).toBe('none');
  });
});

describe.each([false, true])('HUD DOM changes (localising renderer: %s)', (useIsland) => {
  it('leaves identical readouts untouched and changes only the changed heading', () => {
    mountPage();
    if (useIsland) runIsland();
    push({});
    const observer = new MutationObserver(() => {});
    observer.observe(document.body, { subtree: true, childList: true, attributes: true, characterData: true });
    try {
      push({});
      expect(observer.takeRecords()).toEqual([]);
      push({ heading: 91 });
      const changed = observer.takeRecords();
      expect(changed.length).toBeGreaterThan(0);
      expect(changed.every((record) => record.target.id === 'v-nav')).toBe(true);
      expect(text('v-nav')).toBe(useIsland ? t('server.hud_heading', { deg: '091' }) : '091');
      push({ heading: 91 });
      expect(observer.takeRecords()).toEqual([]);
    } finally {
      observer.disconnect();
    }
  });

  it('updates hull, alert and ending transitions without repeating their DOM writes', () => {
    mountPage();
    if (useIsland) runIsland();
    push({});
    const observer = new MutationObserver(() => {});
    observer.observe(document.body, { subtree: true, childList: true, attributes: true, characterData: true });
    try {
      const state = { hull_pct: 12, red_alert: true, condition: 'server.hud_alert', game_over_message: 'The skyway holds.' };
      push(state);
      expect(observer.takeRecords().length).toBeGreaterThan(0);
      expect(text('v-eng')).toBe(useIsland ? t('server.hud_hull', { pct: 12 }) : '12');
      expect(document.getElementById('hud-overlay').classList.contains('alert-on')).toBe(true);
      expect(text('game-over-message')).toBe(state.game_over_message);
      expect(document.getElementById('game-over-overlay').style.display).toBe('flex');
      push(state);
      expect(observer.takeRecords()).toEqual([]);
      push({});
      expect(observer.takeRecords().length).toBeGreaterThan(0);
      expect(document.getElementById('hud-overlay').classList.contains('alert-on')).toBe(false);
      expect(document.getElementById('game-over-overlay').style.display).toBe('none');
      push({});
      expect(observer.takeRecords()).toEqual([]);
    } finally {
      observer.disconnect();
    }
  });
});

describe('the page degrades rather than going blank', () => {
  it('paints a push that landed before the deferred module evaluated', () => {
    // The gap the classic prelude exists to cover: the host pushes as soon as
    // the surface reports loaded, and the island is deferred.
    mountPage();
    push({ heading: 7, game_over_message: 'server.game_over.ship_destroyed' });

    runIsland();
    expect(text('v-nav')).toBe(t('server.hud_heading', { deg: '007' }));
    expect(text('game-over-message')).toBe(t('server.game_over.ship_destroyed'));
    expect(document.getElementById('game-over-overlay').style.display).toBe('flex');
  });

  it('still shows the ending when the module never runs at all', () => {
    // A --client-dir without gui/strings.js, or an engine that refuses the
    // module: the island's imports fail, so it never evaluates and no
    // localising renderer is ever installed. Before the prelude carried a
    // renderer of its own this was a silent no-op — a dead frame over the live
    // scene, with no game-over screen, on the surface the whole room watches.
    mountPage();
    push({ heading: 7, hull_pct: 12, red_alert: true, game_over_message: 'All hands lost.' });

    expect(window.__phoenixHud.render).toBe(null);
    expect(document.getElementById('hud-overlay').classList.contains('alert-on')).toBe(true);
    expect(text('v-nav')).toBe('007');
    expect(text('v-eng')).toBe('12');
    expect(text('game-over-message')).toBe('All hands lost.');
    expect(document.getElementById('game-over-overlay').style.display).toBe('flex');
  });

  it('keeps the markup’s English, and shows digits, when the table did not load', () => {
    // `applyToDom` overwrites each tagged element with t(id), and t() renders
    // an unknown id as ⟨server.hud_nav⟩ — so an unguarded island would turn a
    // failed CSV fetch into a frame full of angle brackets. The same is true of
    // the four runtime values, which is why the guard covers the renderer too.
    //
    // It doubles as the proof that no English is baked into the renderer: a
    // surviving `'HDG ' + heading` would still read "HDG 090" here.
    setTable(new Map());
    mountPage();
    runIsland();
    push({ heading: 90, hull_pct: 84, game_over_message: 'server.game_over.ship_destroyed' });

    expect(slotLabel('server.hud_nav')).toBe('Nav');
    expect(text('hud-designation')).toBe('PHOENIX');
    expect(text('v-nav')).toBe('090');
    expect(text('v-eng')).toBe('84');
    expect(text('v-sys')).toBe('server.hud_nominal');
    expect(document.body.textContent).not.toContain('⟨');
    // An id the table cannot resolve is still shown rather than dropped.
    expect(text('game-over-message')).toBe('server.game_over.ship_destroyed');
  });

  it('survives a malformed push rather than failing it back to the host', () => {
    // A throw would leave `evaluate_script` reporting a failed push, which the
    // host retries with the same payload every frame, forever.
    mountPage();
    runIsland();
    push({ heading: 90 });

    expect(() => window.__updateHud('not json at all')).not.toThrow();
    expect(text('v-nav')).toBe(t('server.hud_heading', { deg: '090' }));
  });
});
