// @vitest-environment jsdom
//
// tests/client/client-lobby-layout.test.js — the client lobby's RESPONSIVE
// contract (issue #1370).
//
// The slice this pins did three things, and none of them was assertable when
// it landed: it deleted the script-computed orientation hook, it introduced
// the `#lobby-side` wrapper that makes the portrait and landscape artboards
// one document, and it put the whole layout switch in a single media query.
// All three are page FACTS — an attribute that must not come back, a nest of
// element ids, a rule in a particular query — and the repo already knows how
// to hold facts like that: design-tokens.test.js, console-tokens.test.js and
// control-floors.test.js all read a document as text and pin what is in it.
//
// Two of these are here because the review of #1370 found the code wrong, not
// because they might one day go wrong:
//
//   - The per-seat job line was dropped for EVERY landscape viewport, which
//     took AC1's "what the seat does" off a 1440x900 desktop — the default
//     host+client session and every Playwright run. Dropping it is a
//     SHORT-viewport measure, so the query that drops it has to constrain
//     height, and the orientation-only query must not mention it at all.
//
//   - The command row's `order` inverted the tab sequence of two buttons.
//     `order` repaints a box without moving its tab stop, so it is safe on
//     the status line (a <span>, no tab stop) and never on a control.
//
// Deliberately NOT here: the 44px touch floor, which control-floors.test.js
// already enforces through its `cursor: pointer` scanner, and the token
// ladder itself, which design-tokens.test.js owns. This file asserts the
// LOBBY's use of both.
import { describe, it, expect } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { REPO_ROOT, GUI, TOKENS_CSS, readStripped, cssRules } from './css-scan.js';

const CLIENT_HTML = path.join(REPO_ROOT, 'client.html');
const RAW = fs.readFileSync(CLIENT_HTML, 'utf8');
/** Comments blanked: a design note that names a deleted hook is worth keeping. */
const SRC = readStripped(CLIENT_HTML);
const RULES = cssRules(SRC);

const doc = new DOMParser().parseFromString(RAW, 'text/html');
const $ = (sel) => doc.querySelector(sel);

/** Every rule whose selector is exactly `sel`, in document order. */
const rulesFor = (sel) => RULES.filter((r) => r.selector.trim() === sel);
/** The one rule for `sel` outside any at-rule. */
const baseRule = (sel) => rulesFor(sel).find((r) => r.at === null);

// ── 1. The dead hook stays dead ─────────────────────────────────────────────

describe('the lobby asks the browser about orientation, not a script', () => {
  // render() used to compute an orientation and write it onto
  // #console-container, where no stylesheet ever selected it — a second answer
  // to a question `@media (orientation: …)` already answers. Re-adding it is
  // how layout creeps back under script, so each half of it is named here.
  it('writes no data-orientation attribute', () => {
    expect(SRC).not.toMatch(/data-orientation/);
  });

  it('loads no gui/device-orientation.js, and the module is gone', () => {
    expect(SRC).not.toMatch(/device-orientation\.js/);
    expect(fs.existsSync(path.join(GUI, 'device-orientation.js'))).toBe(false);
  });

  it('reads no script-held orientation, and re-exports no repaint hook for one', () => {
    // `window.scheduleRender` existed only so the deleted module could coalesce
    // a resize repaint. scheduleRender itself stays, page-local.
    expect(SRC).not.toMatch(/currentOrientation/);
    expect(SRC).not.toMatch(/window\.scheduleRender/);
  });
});

// ── 2. One document at two frame sizes ──────────────────────────────────────

describe('#lobby-side is the joint the two artboards share', () => {
  it('wraps the seat detail and the command row inside the lobby body', () => {
    const body = $('#lobby-body');
    expect(body).not.toBeNull();
    const side = body.querySelector(':scope > #lobby-side');
    expect(side).not.toBeNull();
    expect(body.querySelector(':scope > #station-list')).not.toBeNull();
    expect(side.querySelector('#detail-panel')).not.toBeNull();
    expect(side.querySelector('#lobby-footer')).not.toBeNull();
  });

  it('dissolves in portrait and stands up as a rail in landscape', () => {
    // `display: contents` is what makes the wrapper free: the detail card and
    // the Ready bar flow as the column's own children, exactly as three
    // stacked siblings would, and the SAME markup becomes a rail one query
    // later. If the base rule ever stops being `contents`, portrait has
    // quietly grown a box the artboard does not have.
    expect(baseRule('#lobby-side').body).toMatch(/display:\s*contents/);

    const inLandscape = rulesFor('#lobby-side').filter((r) => r.at !== null);
    expect(inLandscape).toHaveLength(1);
    expect(inLandscape[0].at).toMatch(/@media\s*\(orientation:\s*landscape\)/);
    expect(inLandscape[0].body).toMatch(/display:\s*(?!contents)/);
  });
});

// ── 3. The job line is dropped on HEIGHT, never on orientation alone ────────

describe('the roster keeps telling a player what a seat does', () => {
  const descRules = RULES.filter(
    (r) => /\.station-row[^,{]*\.desc\b/.test(r.selector) && /display:\s*none/.test(r.body),
  );

  it('never hides the job line in a query that only asks about orientation', () => {
    // AC1 has no orientation qualifier, PRD #1355's user story 47 is reading
    // the job BEFORE claiming, and a landscape desktop has hundreds of spare
    // pixels. Whatever hides it must also be asking how TALL the viewport is.
    for (const r of descRules) {
      expect(r.at, `${r.selector.trim()} hides the job line under ${r.at}`)
        .toMatch(/max-height/);
    }
  });

  it('leaves the job line alone in the layout query itself', () => {
    const layoutQuery = RULES.filter(
      (r) => r.at !== null && /orientation:\s*landscape/.test(r.at) && !/height/.test(r.at),
    );
    expect(layoutQuery.length).toBeGreaterThan(0);
    for (const r of layoutQuery) expect(r.selector).not.toMatch(/\.desc\b/);
  });
});

// ── 4. The lobby's rules stay in the lobby ──────────────────────────────────

describe('the landscape block does not restyle the spectator claim list', () => {
  it('scopes every .station-row rule it writes to #lobby-ui', () => {
    // #spectator-claims (issue #1106) reuses .station-row for its claim rows.
    // A compaction drawn for the lobby's roster reaching a second surface is
    // the kind of change nobody goes looking for.
    const rows = RULES.filter(
      (r) => r.at !== null && /orientation:\s*landscape/.test(r.at)
        && /\.station-row\b/.test(r.selector),
    );
    expect(rows.length).toBeGreaterThan(0);
    for (const r of rows) expect(r.selector.trim()).toMatch(/^#lobby-ui\s/);
  });
});

// ── 5. Focus order follows paint order ──────────────────────────────────────

describe('the command row can be tabbed through in the order it is read', () => {
  it('paints the two buttons in DOM order, so `order` moves no tab stop', () => {
    // `order` repaints a box and leaves its tab stop where the markup put it
    // (WCAG 2.4.3 / 1.3.2). READY paints left of SPECTATE, so READY is first
    // in the markup, and neither rule carries an `order`.
    const ids = [...$('#lobby-footer').querySelectorAll('button')].map((b) => b.id);
    expect(ids).toEqual(['ready-btn', 'spectate-btn']);
    for (const sel of ['#ready-btn', '#spectate-btn']) {
      expect(baseRule(sel).body, `${sel} reorders a focusable control`)
        .not.toMatch(/(^|[^-\w])order\s*:/);
    }
  });

  it('reorders only the status line, which has no tab stop to move', () => {
    // First in the markup so a screen reader hears the connection state before
    // the control it gates; painted last because that is where the artboard
    // puts it. A <span> with no tabindex costs nothing to move.
    const status = $('#status-line');
    expect(status.tagName).toBe('SPAN');
    expect(status.hasAttribute('tabindex')).toBe(false);
    expect(status.compareDocumentPosition($('#ready-btn'))
      & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(baseRule('#status-line').body).toMatch(/(^|[^-\w])order\s*:/);
  });
});

// ── 6. Control boundaries take the ladder rung for their own fill ───────────

describe("the lobby's controls sit on gui/tokens.css's control ladder", () => {
  const TOKENS = fs.readFileSync(TOKENS_CSS, 'utf8');
  const hex = (name) => {
    const m = TOKENS.match(new RegExp(`${name}:\\s*(#[0-9a-fA-F]{6})`));
    return m ? m[1] : null;
  };
  const channel = (c) => {
    const v = c / 255;
    return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
  };
  const luminance = (h) => {
    const [r, g, b] = [1, 3, 5].map((i) => channel(parseInt(h.slice(i, i + 2), 16)));
    return 0.2126 * r + 0.7152 * g + 0.0722 * b;
  };
  const ratio = (a, b) => {
    const [la, lb] = [luminance(a), luminance(b)];
    return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
  };

  /**
   * The ENABLED controls of the lobby that are drawn as a filled box with a
   * boundary. Each is read out of the page rather than written down here, so
   * changing either the fill or the boundary re-runs the measurement — which
   * is the whole point: three of these took a rung chosen for a different
   * surface, and every one of them looked fine in the token's own quoted
   * ratio, which is measured against the reference GROUND.
   *
   * Not listed, with reasons: `.taken-btn` and `.ineligible-btn`, which are
   * disabled (`cursor: default` / `not-allowed`) and exempt from 1.4.11; the
   * `.claim-btn`/`.mine-btn` accent pair, whose boundary is --cyan.
   */
  const CONTROLS = [
    '#detail-panel .detail-consoles .chip',
    '#detail-panel .detail-ratings .rating-btn',
    '#ready-btn',
    '#spectate-btn',
  ];

  for (const sel of CONTROLS) {
    it(`${sel} clears WCAG 1.4.11's 3:1 against the fill it encloses`, () => {
      const rule = baseRule(sel);
      expect(rule, `no rule for ${sel}`).toBeDefined();
      const fill = rule.body.match(/background(?:-color)?:\s*var\((--surface-[a-z-]+)\)/);
      const edge = rule.body.match(/border(?:-color)?:\s*(?:1px solid\s+)?var\((--edge-[a-z-]+)\)/);
      expect(fill, `${sel} has no token fill`).not.toBeNull();
      expect(edge, `${sel} has no token boundary`).not.toBeNull();
      const f = hex(fill[1]);
      const e = hex(edge[1]);
      expect(f, `${fill[1]} is not a hex in tokens.css`).not.toBeNull();
      expect(e, `${edge[1]} is not a hex in tokens.css`).not.toBeNull();
      expect(
        ratio(f, e),
        `${edge[1]} on ${fill[1]} — step up the ladder in gui/tokens.css`,
      ).toBeGreaterThanOrEqual(3);
    });
  }
});
