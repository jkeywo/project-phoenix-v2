import { test, expect } from '@playwright/test';

/**
 * AC1 of issue #1375, measured: "at 375x812, 812x375 and desktop the scope is
 * square, the circle touches the edges, and no chrome overlaps the contacts".
 *
 * tests/client/radar-scope-contract.test.js pins the RULES that are supposed
 * to produce that — one `.scope-cell` declaration, no per-console
 * aspect-ratio, the two rail custom properties. It cannot pin the result:
 * jsdom has no layout engine, so every one of those assertions passed
 * unchanged while destroyer/captain rendered a 66x66 scope at 568x320 and
 * 148x148 at 650x450. This spec is the half that needs a browser.
 *
 * Three assertions per scope, and all three are load-bearing:
 *
 *   square — |width - height| <= 1. The design rule.
 *   floor  — at least a third of the smaller viewport edge. Squareness alone
 *            is happy with 66x66; the floor is what catches a rail taking the
 *            whole row and leaving the picture a postage stamp. A third is
 *            deliberately generous: every console clears it with room to
 *            spare, so a failure means something structural, not a few pixels
 *            of padding drift.
 *   clear  — the second clause of AC1: nothing a rail contains may be drawn
 *            over the scope. That is what a fixed-size widget in a shrinking
 *            rail does — the joystick disc spills sideways rather than the
 *            column clipping it — and it is invisible to a size assertion,
 *            because the scope is the right size and the contacts are under a
 *            control anyway.
 *
 * And three per DOCUMENT, about what the square displaced. A scope that claims
 * more of the page pushes everything else along, and none of the three
 * measurements above moves when it pushes a control off the bottom edge: they
 * all describe the scope. Round 2 of this slice shipped exactly that — all ten
 * documents green here while destroyer/helm rendered IMPULSE and BOOST at
 * 746..827 in an 812px viewport with `body { overflow: hidden }` over them.
 *
 *   reachable — every visible control ends on the page, or inside something
 *               that scrolls. A control below the fold is fine when a portrait
 *               console has made `.console-body` a scroller (battleship/sensors,
 *               destroyer/captain, both helms); it is a bug when nothing
 *               between it and the document scrolls, because then `body
 *               { overflow: hidden }` is simply cutting it off.
 *   squeezed  — no visible control is laid out at zero height around content it
 *               still holds. One collapse further on than `reachable`: a
 *               control the square has pressed flat is not below the fold, it
 *               is nowhere, and the column's `overflow-y: auto` has nothing to
 *               scroll to because a zero-height element adds nothing to scroll
 *               past. Round 3 of this slice shipped cruiser/science with
 *               `ph-sensor-panel#sensor-panel` zero tall around 52px of SCAN
 *               RANGE / CONTACTS / NO TARGET — green here, because the
 *               reachability loop skipped anything measuring zero as though it
 *               were hidden.
 *   contained — every `.scope-row` child holds its own content, unless it is a
 *               scroller itself. A column shorter than what is inside it does
 *               not clip: the widget paints on past the border and over its
 *               neighbour, which is the same failure as `clear` above seen
 *               from the rail's side rather than the scope's.
 *
 * 650x450 is in the list because it is the size this codebase has caught
 * layout regressions at before, and 1280x720 stands in for a desktop.
 */

// Every console document with a scope on it, and the scope elements to find.
// One list rather than a per-hull spec: the point of #1375 is that all ten
// obey the SAME rule, so a hull that stops doing so should fail here rather
// than in a suite somebody forgot to add it to.
const SCOPE_DOCS = [
  'battleship/helm.html',
  'battleship/sensors.html',
  'battleship/tactical.html',
  'courier/tactical.html',
  'cruiser/helm.html',
  'cruiser/science.html',
  'cruiser/tactical.html',
  'destroyer/captain.html',
  'destroyer/helm.html',
  'destroyer/tactical.html',
];

const SCOPE_SELECTOR = 'ph-helm-radar, ph-tactical-radar, ph-sensor-radar';

const VIEWPORTS = [
  { width: 375, height: 812 },
  { width: 812, height: 375 },
  { width: 650, height: 450 },
  { width: 1280, height: 720 },
];

// What a console asks a crew member to reach for. Named elements rather than
// "everything visible" because a readout scrolled off the bottom of a portrait
// stack is a nuisance and a BOOST button scrolled off it is a ship that will
// not accelerate — and because a list of tags fails loudly when a hull grows a
// control nobody added here, which is the reminder we want.
const CONTROL_SELECTOR = [
  'button',
  'ph-impulse-btn',
  'ph-boost-btn',
  'ph-helm-joystick',
  'ph-lateral-thrust-joystick',
  'ph-phasers-controls',
  'ph-blasters-controls',
  'ph-torpedo-controls',
  'ph-sensor-panel',
].join(', ');

/** Measure every scope in the open document, and what overlaps it. */
async function measureScopes(page, selector) {
  return page.evaluate((sel) => {
    const boxOf = (el) => {
      const r = el.getBoundingClientRect();
      return { left: r.left, top: r.top, right: r.right, bottom: r.bottom, width: r.width, height: r.height };
    };
    // Anything a rail draws that lands inside the scope's box. Names the
    // OUTERMOST element that overlaps and stops there, descending only where
    // an ancestor's own box clears the scope — which is precisely the failure
    // this is here for: a fixed-size widget spilling out of a rail too narrow
    // for it, where the column is innocent and the disc inside it is not. A
    // column whose own box laps over the square is named as the column,
    // because at that point the column is the thing that is wrong.
    const intruders = (scope) => {
      const cell = scope.closest('.scope-cell') || scope.parentElement;
      const row = cell.parentElement;
      const box = boxOf(cell);
      const found = [];
      const walk = (el) => {
        const r = boxOf(el);
        if (r.width === 0 || r.height === 0) return;
        const overX = Math.min(r.right, box.right) - Math.max(r.left, box.left);
        const overY = Math.min(r.bottom, box.bottom) - Math.max(r.top, box.top);
        if (overX > 1 && overY > 1) {
          found.push(`${el.tagName.toLowerCase()} over the scope by ${Math.round(overX)}x${Math.round(overY)}px`);
          return;
        }
        for (const child of el.children) walk(child);
      };
      for (const sibling of row.children) if (sibling !== cell) walk(sibling);
      return found;
    };
    return [...document.querySelectorAll(sel)].map((el) => ({
      tag: el.tagName.toLowerCase(),
      id: el.id,
      width: Math.round(el.getBoundingClientRect().width),
      height: Math.round(el.getBoundingClientRect().height),
      intruders: intruders(el),
    }));
  }, selector);
}

/**
 * Measure what the scope displaced: controls pushed off the page with no way
 * to reach them, controls squeezed flat, and `.scope-row` children too small
 * for what they hold.
 */
async function measureFit(page, controlSelector) {
  return page.evaluate((sel) => {
    // A scroller is an escape hatch: content below the fold inside one is
    // reachable, so it does not count as cut off. `scrollHeight > clientHeight`
    // as well as the overflow value, because a box declared `overflow-y: auto`
    // that has nothing to scroll offers no escape either.
    const scrolls = (el) => {
      const overflowY = getComputedStyle(el).overflowY;
      if (overflowY !== 'auto' && overflowY !== 'scroll') return false;
      return el.scrollHeight > el.clientHeight + 1;
    };
    const reachable = (el) => {
      for (let p = el.parentElement; p; p = p.parentElement) if (scrolls(p)) return true;
      return false;
    };

    // Hidden is not the same as squeezed, and conflating the two is what let
    // ph-sensor-panel ship zero tall on cruiser/science. A console deliberately hides a control it
    // has nothing to say about — destroyer/helm's dock panel and tow banner are
    // `[hidden]`, so are their buttons by inheritance — and that control is not
    // this spec's business. A control the square has pressed flat is displayed,
    // is in the layout, and is still something a crew member is meant to reach;
    // it just has nowhere to be. So test for hiding directly rather than
    // inferring it from a zero box.
    const hidden = (el) => {
      if (el.hidden || el.closest('[hidden]')) return true;
      const cs = getComputedStyle(el);
      if (cs.display === 'none' || cs.visibility === 'hidden' || cs.visibility === 'collapse') return true;
      // `display: none` on an ANCESTOR leaves this element's own computed
      // display alone but takes its box out of the layout entirely, so it
      // generates no client rects. A squeezed element still generates one —
      // full width and zero tall — which is how the two are told apart.
      return el.getClientRects().length === 0;
    };
    const nameOf = (el) => el.tagName.toLowerCase() + (el.id ? `#${el.id}` : '');

    const unreachable = [];
    const squeezed = [];
    for (const el of document.querySelectorAll(sel)) {
      if (hidden(el)) continue;
      const r = el.getBoundingClientRect();
      if (r.height <= 1 && el.scrollHeight > 1) {
        squeezed.push(`${nameOf(el)} is laid out ${Math.round(r.width)}x0 around ${el.scrollHeight}px of content`);
      }
      if (r.bottom <= innerHeight + 1) continue;
      if (reachable(el)) continue;
      unreachable.push(`${nameOf(el)} ends at ${Math.round(r.bottom)}, past the ${innerHeight}px page, with nothing to scroll`);
    }

    const spilling = [];
    for (const row of document.querySelectorAll('.scope-row')) {
      for (const child of row.children) {
        const overflowY = getComputedStyle(child).overflowY;
        if (overflowY === 'auto' || overflowY === 'scroll') continue; // clips and scrolls, by design
        if (child.scrollHeight <= child.clientHeight + 1) continue;
        const name = child.className || child.tagName.toLowerCase();
        spilling.push(`${name} holds ${child.scrollHeight}px of content in a ${child.clientHeight}px box`);
      }
    }
    return { unreachable, squeezed, spilling };
  }, controlSelector);
}

for (const viewport of VIEWPORTS) {
  const label = `${viewport.width}x${viewport.height}`;
  test(`every scope is a square that fills its slot at ${label}`, { tag: '@core' }, async ({ page }) => {
    // The floor: a third of the smaller viewport edge. Not a pixel budget —
    // a "did the layout give the picture a share of the screen at all" check.
    const floor = Math.min(viewport.width, viewport.height) / 3;
    await page.setViewportSize(viewport);

    for (const doc of SCOPE_DOCS) {
      await page.goto(`/gui/${doc}`);
      // The consoles are pure JS: the scope is a custom element that upgrades
      // on its module's import, so wait for a laid-out box rather than for
      // the markup, which is there before the element exists.
      await page.waitForFunction(
        (sel) => {
          const els = [...document.querySelectorAll(sel)];
          return els.length > 0 && els.every((el) => el.shadowRoot);
        },
        SCOPE_SELECTOR,
        { timeout: 10_000 },
      );

      const scopes = await measureScopes(page, SCOPE_SELECTOR);
      expect(scopes.length, `${doc} has no scope element`).toBeGreaterThan(0);

      for (const scope of scopes) {
        const where = `${doc} ${scope.tag}#${scope.id} at ${label}`;
        expect(
          Math.abs(scope.width - scope.height),
          `${where} is ${scope.width}x${scope.height}, not square`,
        ).toBeLessThanOrEqual(1);
        expect(
          scope.width,
          `${where} is ${scope.width}px, under the ${Math.round(floor)}px floor`,
        ).toBeGreaterThanOrEqual(floor);
        expect(scope.intruders, `${where}: ${scope.intruders.join(', ')}`).toEqual([]);
      }

      // What the square pushed out of the way. Per document rather than per
      // scope: the displaced control is somewhere else on the page entirely.
      const fit = await measureFit(page, CONTROL_SELECTOR);
      expect(
        fit.unreachable,
        `${doc} at ${label} cuts off: ${fit.unreachable.join('; ')}`,
      ).toEqual([]);
      expect(
        fit.squeezed,
        `${doc} at ${label} flattens: ${fit.squeezed.join('; ')}`,
      ).toEqual([]);
      expect(
        fit.spilling,
        `${doc} at ${label} paints outside a column: ${fit.spilling.join('; ')}`,
      ).toEqual([]);
    }
  });
}
