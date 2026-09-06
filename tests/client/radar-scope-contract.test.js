// @vitest-environment jsdom
/**
 * tests/client/radar-scope-contract.test.js — module 2's draw contract (PRD #1023).
 *
 * The PRD asks for three things of the hardened scope, and names the test for
 * each: "rings present at the configured ranges, labels haloed and
 * non-overlapping for a clustered fixture, arc alpha capped under stacked
 * overlays". Those are the first three describes below.
 *
 * The fourth is issue #1375's, and it is the SHAPE contract rather than the
 * draw contract: a scope is a square whose circle touches its edges, so its
 * chrome lives in the four corners the circle cannot reach and the scale
 * readout stays on the ring between them. Those assertions read the mounted
 * shadow DOM, the console documents on disk and the same draw log, for the
 * same reason — a scope that has stopped being square still calls every
 * function it called yesterday.
 *
 * Every assertion reads the DRAW LOG or the rendered attributes — the radii the
 * scope actually stroked, the coordinates it actually painted text at, the
 * opacity a player's eye actually receives. None of them reaches for a private
 * method or asserts that some helper was called, because the failures these
 * exist to catch are failures of the picture: a ring at the wrong radius still
 * calls the same function.
 *
 * Geometry, once, so the numbers below are readable: the scope is 300 CSS px
 * square with a 2× backing store, so the canvas is 600 × 600, its centre is
 * (300, 300) and its radius R is 300 BUFFER pixels. `px` — buffer pixels per
 * CSS pixel — is therefore 2, and every authored size is multiplied by it.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { t } from '../../gui/strings.js';
import '../../gui/components/ph-radar.js';
import '../../gui/components/ph-tactical-radar.js';
import '../../gui/components/ph-helm-radar.js';
import '../../gui/components/ph-sensor-radar.js';
import { makeRadarCtx, strokedArcRadii, paintedText, haloedText } from './radar-canvas-stub.js';
import { ringPlan, ringStep, ARC_COMPOSITE_MAX } from '../../gui/components/ph-scope-chrome.js';
import {
  GUI, readStripped, cssRules, consoleDocuments, rel,
} from './css-scan.js';

const CSS_SIZE = 300;
const DPR = 2;
const R = 300;         // buffer pixels
const PX = 2;          // buffer pixels per CSS pixel
const TEXT_MIN = 11;   // --text-min; jsdom loads no stylesheet, so the fallback
const FONT = TEXT_MIN * PX;
const LINE_HEIGHT = FONT * 1.2;

let fakeCtx;
let rafCb;
let origGetContext;
let origRAF;
let origCARAF;
let origRO;
let origImage;

beforeEach(() => {
  fakeCtx = makeRadarCtx();
  origGetContext = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = function () { return fakeCtx; };
  origRAF = window.requestAnimationFrame;
  // The scope renders from its own rAF loop, so a frame is something the test
  // drives rather than something that happens: the callback is captured here
  // and invoked by `draw()` below.
  window.requestAnimationFrame = vi.fn((cb) => { rafCb = cb; return 1; });
  origCARAF = window.cancelAnimationFrame;
  window.cancelAnimationFrame = vi.fn();
  origRO = window.ResizeObserver;
  window.ResizeObserver = function () {
    return { observe: vi.fn(), disconnect: vi.fn() };
  };
  origImage = window.Image;
  // Deliberately NEVER "loaded": with the background PNGs absent the scope
  // paints its fallback disc, which keeps the draw log to the geometry these
  // tests are about rather than three drawImage calls of chrome.
  window.Image = class {
    constructor() { this.naturalWidth = 0; this.complete = false; }
    set src(v) { this._src = v; }
    get src() { return this._src; }
  };
  Object.defineProperty(window, 'devicePixelRatio', { value: DPR, configurable: true });
});

afterEach(() => {
  HTMLCanvasElement.prototype.getContext = origGetContext;
  window.requestAnimationFrame = origRAF;
  window.cancelAnimationFrame = origCARAF;
  window.ResizeObserver = origRO;
  window.Image = origImage;
  document.body.innerHTML = '';
  delete window.sendAction;
});

/** A mounted `<ph-radar>` sized as described in the header, plus a manual tick. */
function scope() {
  document.body.innerHTML = '<ph-radar id="scope"></ph-radar>';
  const el = document.getElementById('scope');
  const canvas = el.shadowRoot.querySelector('canvas');
  const rect = {
    width: CSS_SIZE, height: CSS_SIZE, left: 0, top: 0,
    right: CSS_SIZE, bottom: CSS_SIZE,
  };
  vi.spyOn(el, 'getBoundingClientRect').mockReturnValue(rect);
  vi.spyOn(canvas, 'getBoundingClientRect').mockReturnValue(rect);
  canvas.width = CSS_SIZE * DPR;
  canvas.height = CSS_SIZE * DPR;

  return {
    el,
    /**
     * Set state, run one frame, and return the draw log for THAT frame alone.
     *
     * The log is cleared first so an assertion never reads a ring the previous
     * state drew — which matters most for the test that changes the range and
     * expects the old scale to be gone.
     */
    draw(state) {
      el.state = state;
      fakeCtx._reset();
      const frame = rafCb;
      rafCb = null;
      if (frame) frame();
      return fakeCtx;
    },
  };
}

// ── 1. Rings, and the readout that says what they measure ──────────────────

describe('the scope draws range rings at the ranges it was configured with', () => {
  it('spaces rings on a round ladder rather than at fixed fractions', () => {
    // gui/radar-widget.js drew 33 / 66 / 100 % of the radius whatever the
    // range, so the middle ring stood for 333 units on a 500 scope and 200 on
    // a 300 one — a picture that measured nothing. These are distances a
    // player can name.
    expect(ringStep(500)).toBe(100);
    expect(ringStep(300)).toBe(100);
    expect(ringPlan(500).map((r) => r.distance)).toEqual([100, 200, 300, 400, 500]);
    expect(ringPlan(300).map((r) => r.distance)).toEqual([100, 200, 300]);
  });

  it('strokes a ring at each planned distance, scaled to the scope radius', () => {
    const s = scope();
    const ctx = s.draw({ range: 500, blips: [] });
    // 100 / 200 / 300 / 400 / 500 of 500, times R.
    expect(strokedArcRadii(ctx)).toEqual([60, 120, 180, 240, 300]);
  });

  it('always closes the scope with a ring at its own edge', () => {
    // Without it a contact clamped to the rim looks like it is floating
    // outside the picture. 300 / 100 does not divide evenly, so this is the
    // case where the outer ring is not a multiple of the step.
    const s = scope();
    const ctx = s.draw({ range: 250, blips: [] });
    expect(strokedArcRadii(ctx).at(-1)).toBe(R);
  });

  it('says what the outer ring means', () => {
    const s = scope();
    const ctx = s.draw({ range: 500, blips: [] });
    const readout = paintedText(ctx).map((p) => p.text);
    expect(readout).toContain(t('console.radar.scale', { range: '500' }));
  });

  it('REDRAWS the scale when damage or doctrine changes the range', () => {
    // The helm scope's range is `bb.radar_range`, republished every tick and
    // shrunk by `apply_radar_damage_modifiers` as the radar system is hit. A
    // scope that kept painting the range it booted with would be lying at
    // exactly the moment the crew most need it not to.
    const s = scope();
    const full = paintedText(s.draw({ range: 500, blips: [] })).map((p) => p.text);
    const degraded = paintedText(s.draw({ range: 300, blips: [] })).map((p) => p.text);

    expect(full).toContain(t('console.radar.scale', { range: '500' }));
    expect(degraded).toContain(t('console.radar.scale', { range: '300' }));
    expect(degraded).not.toContain(t('console.radar.scale', { range: '500' }));

    // And the rings move with it. Note what does NOT follow: halving the range
    // halves the step too, so a 500 scope and a 250 one draw rings at
    // IDENTICAL radii and differ only in what those radii mean. That is the
    // readout earning its place — without it the scope's picture is the same
    // at both ranges and the crew have no way to tell which they are looking
    // at. 500 → 300 is a case where the ring count itself changes.
    expect(strokedArcRadii(s.draw({ range: 500, blips: [] })).length).toBe(5);
    expect(strokedArcRadii(s.draw({ range: 300, blips: [] })).length).toBe(3);
    expect(strokedArcRadii(s.draw({ range: 250, blips: [] })))
      .toEqual(strokedArcRadii(s.draw({ range: 500, blips: [] })));
  });

  it('still draws rings, and claims no scale, when it is handed no range', () => {
    // A scope with no range is a scope that cannot label a ring. Rings without
    // numbers are a weaker picture; numbers without a range behind them would
    // be a false one.
    const s = scope();
    const ctx = s.draw({ blips: [] });
    expect(strokedArcRadii(ctx)).toEqual([R * 0.33, R * 0.66, R]);
    expect(paintedText(ctx)).toEqual([]);
  });
});

// ── 2. Labels: haloed, and clear of each other ─────────────────────────────

/** The box a label occupies, in buffer pixels. */
function labelBox(painted, ctx) {
  ctx.font = FONT + 'px x';
  return {
    text: painted.text,
    left: painted.x,
    right: painted.x + ctx.measureText(painted.text).width,
    top: painted.y - LINE_HEIGHT,
    bottom: painted.y,
  };
}

function overlaps(a, b) {
  return a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top;
}

/**
 * The contact labels only.
 *
 * The scale readout is painted with the same call, and it is deliberately NOT
 * part of the de-collision pass: it is anchored to the scope's RIM rather than
 * to a contact, so a contact label crowding it is the scope being full, not a
 * de-collision failure. Since issue #1375 it is drawn left-aligned from a
 * bottom baseline on the aft-port diagonal -- exactly the geometry `labelBox`
 * above models -- so the clearance it does owe is to the speed readout sharing
 * that corner, and `the scale readout stays on the ring, clear of all four
 * corners` below is where that is pinned.
 */
function contactLabels(ctx, fixture) {
  const wanted = new Set(fixture.blips.map((b) => b.label));
  return paintedText(ctx).filter((p) => wanted.has(p.text));
}

/** Four contacts inside a few pixels of each other — a furball. */
const CLUSTER = {
  range: 500,
  blips: [
    { uuid: 'a', radar_x: 0, radar_y: 0.00, label: 'HARROW' },
    { uuid: 'b', radar_x: 0, radar_y: 0.01, label: 'MERIDIAN' },
    { uuid: 'c', radar_x: 0, radar_y: 0.02, label: 'CASTELLAN' },
    { uuid: 'd', radar_x: 0, radar_y: 0.03, label: 'SKYWAY' },
  ],
};

describe('clustered contact labels stay readable', () => {
  it('paints every label the state asked for', () => {
    const s = scope();
    const painted = paintedText(s.draw(CLUSTER)).map((p) => p.text);
    for (const blip of CLUSTER.blips) expect(painted).toContain(blip.label);
  });

  it('haloes each label so it survives whatever backdrop it lands on', () => {
    // The scope's backdrop is a photographic PNG. A label's contrast against
    // it is otherwise whatever pixels it happens to fall on.
    const ctx = scope().draw(CLUSTER);
    const halos = haloedText(ctx);
    const fills = paintedText(ctx);
    expect(halos.length).toBe(fills.length);
    // Same glyphs at the same place — an outline behind the text, not a second
    // string somewhere near it.
    expect(halos).toEqual(fills);
  });

  it('nudges the labels apart so no two overlap', () => {
    const ctx = scope().draw(CLUSTER);
    const boxes = contactLabels(ctx, CLUSTER).map((p) => labelBox(p, ctx));
    expect(boxes.length).toBe(CLUSTER.blips.length);
    const collisions = [];
    for (let i = 0; i < boxes.length; i += 1) {
      for (let j = i + 1; j < boxes.length; j += 1) {
        if (overlaps(boxes[i], boxes[j])) {
          collisions.push(`${boxes[i].text} over ${boxes[j].text}`);
        }
      }
    }
    expect(collisions).toEqual([]);
  });

  it('leaves a label alone when nothing is near it', () => {
    // De-collision that moves labels it did not need to move is its own bug:
    // a label away from its blip is a label pointing at the wrong contact.
    const s = scope();
    const lone = s.draw({
      range: 500,
      blips: [{ uuid: 'a', radar_x: 0, radar_y: 0, label: 'HARROW' }],
    });
    const [painted] = paintedText(lone).filter((p) => p.text === 'HARROW');
    // by = 300 - 0 = 300; dotR = max(6 × 2, 0) = 12; the label sits 4 CSS px
    // beyond the blip on both axes.
    expect(painted.x).toBeCloseTo(300 + 12 + 4 * PX, 6);
    expect(painted.y).toBeCloseTo(300 + 4 * PX, 6);
  });

  it('sizes labels from the type floor, not from a number in the render call', () => {
    // `--text-min` is the one definition of "the smallest a string may render".
    // The scope draws into a buffer sized rect × DPR, so the floor reaches the
    // canvas multiplied by that ratio and no other way.
    const ctx = scope().draw(CLUSTER);
    expect(ctx.font).toBe(FONT + 'px "JetBrains Mono", monospace');
  });
});

// ── 3. Stacked firing arcs stay translucent ────────────────────────────────

function tacticalScope() {
  document.body.innerHTML = '<ph-tactical-radar id="tac"></ph-tactical-radar>';
  const el = document.getElementById('tac');
  vi.spyOn(el, 'getBoundingClientRect').mockReturnValue({
    width: CSS_SIZE, height: CSS_SIZE, left: 0, top: 0,
    right: CSS_SIZE, bottom: CSS_SIZE,
  });
  return el;
}

/**
 * What a stack of `n` overlays composites to.
 *
 * Translucent fills accumulate rather than average: `1 - (1 - a)^n`. The group
 * opacity then multiplies that flattened result exactly once, however many
 * children produced it — which is the whole reason the cap lives on the group.
 */
function stackedAlpha(group) {
  const groupOpacity = parseFloat(group.getAttribute('opacity') ?? '1');
  let transparent = 1;
  for (const path of group.querySelectorAll('path')) {
    transparent *= 1 - parseFloat(path.getAttribute('fill-opacity') ?? '1');
  }
  return (1 - transparent) * groupOpacity;
}

describe('overlapping firing arcs stay translucent however many stack', () => {
  const bank = (facing) => ({ facing_deg: facing, arc_deg: 300 });

  it('paints one arc at exactly the alpha it was authored with', () => {
    const el = tacticalScope();
    el.state = { phaser_arcs: [{ ...bank(0), opacity: 0.3 }] };
    const g = el.shadowRoot.getElementById('phaser-arcs');
    expect(stackedAlpha(g)).toBeCloseTo(0.3, 6);
  });

  it('holds the composite under the cap with eight banks over the same pixel', () => {
    // Eight 300° banks all cover the ship's forward quarter, so this is a real
    // overlap, not a contrived one. Uncapped, eight arcs at 0.3 composite to
    // 0.94 — an opaque wall over the contact the officer is about to shoot.
    const el = tacticalScope();
    el.state = {
      phaser_arcs: Array.from({ length: 8 }, (_, i) => ({ ...bank(i * 45), opacity: 0.3 })),
    };
    const g = el.shadowRoot.getElementById('phaser-arcs');
    expect(g.querySelectorAll('path').length).toBe(8);
    expect(stackedAlpha(g)).toBeLessThanOrEqual(ARC_COMPOSITE_MAX);
    expect(stackedAlpha(g)).toBeGreaterThan(0.3);
  });

  it('caps the composite whatever the authored alpha, even at full opacity', () => {
    // The cap is a ceiling on the picture, not a scaling of the input: an arc
    // the server marked opaque still cannot black out the scope.
    const el = tacticalScope();
    el.state = {
      phaser_arcs: Array.from({ length: 4 }, (_, i) => ({ ...bank(i * 90), opacity: 1 })),
    };
    const g = el.shadowRoot.getElementById('phaser-arcs');
    expect(stackedAlpha(g)).toBeCloseTo(ARC_COMPOSITE_MAX, 6);
  });

  it('caps every arc group, not just the one that was noticed first', () => {
    const el = tacticalScope();
    for (const id of ['phaser-arcs', 'torpedo-arcs']) {
      const g = el.shadowRoot.getElementById(id);
      expect(parseFloat(g.getAttribute('opacity'))).toBe(ARC_COMPOSITE_MAX);
    }
  });

  it('keeps a fainter bank fainter than a stronger one', () => {
    // A cap that flattened every arc to the same alpha would throw away the
    // weighting the server authored.
    const el = tacticalScope();
    el.state = {
      phaser_arcs: [{ ...bank(0), opacity: 0.1 }, { ...bank(180), opacity: 0.4 }],
    };
    const [faint, strong] = el.shadowRoot.getElementById('phaser-arcs')
      .querySelectorAll('path');
    expect(parseFloat(faint.getAttribute('fill-opacity')))
      .toBeLessThan(parseFloat(strong.getAttribute('fill-opacity')));
  });
});

// ── 4. A square with four dead corners (issue #1375) ───────────────────────
//
// The scope is a circle inscribed in a square whose edges it touches. That
// makes the four corners the only part of the cell the picture never reaches,
// and therefore the only place chrome can stand without covering a contact:
// three readouts and, where a console has one, ON SCREEN. The scale readout is
// the exception that proves the rule — it names the outer ring, so it stays ON
// the ring, and the corners are what it has to keep clear of.

/** The four corner slots, in the order a reader would name them. */
const CORNERS = ['top-left', 'top-right', 'bottom-left', 'bottom-right'];

/** Mount one scope element and hand back its shadow root. */
function mount(tag) {
  document.body.innerHTML = '<' + tag + ' id="scope-under-test"></' + tag + '>';
  return document.getElementById('scope-under-test').shadowRoot;
}

/** Which corner slot each piece of chrome in this scope occupies. */
function cornersOf(root) {
  return [...root.querySelectorAll('.scope-corner')].map((el) => ({
    id: el.id,
    corner: CORNERS.filter((c) => el.classList.contains(c)),
  }));
}

describe('a scope keeps its chrome in the corners the circle cannot reach', () => {
  it('puts the three readouts and ON SCREEN in four DISTINCT corners', () => {
    const chrome = cornersOf(mount('ph-helm-radar'));
    // Every piece names exactly one slot — a corner class is a position, and
    // two of them on one element is two positions.
    for (const piece of chrome) expect(piece.corner.length).toBe(1);
    expect(chrome.map((p) => p.id + '@' + p.corner[0]).sort()).toEqual([
      'label-bearing@top-right',
      'label-pos@top-left',
      'label-speed@bottom-left',
      'on-screen-btn@bottom-right',
    ]);
  });

  it('gives the sensors scope the same four corners, from the same fragment', () => {
    // Sensors used to carry its own byte-identical copy of the ON SCREEN block.
    // Two copies of one control is how the two scopes' fourth corners end up in
    // different places, so the test is that they agree — not merely that each
    // has a button somewhere.
    const helm = cornersOf(mount('ph-helm-radar'));
    const sensors = cornersOf(mount('ph-sensor-radar'));
    expect(sensors).toEqual(helm);
  });

  it('leaves the fourth corner EMPTY on a scope with no ON SCREEN', () => {
    // A weapons officer does not choose the viewscreen, so the tactical scope
    // has no such button — and the shared fragment must not hand it one for
    // the sake of symmetry.
    const root = mount('ph-tactical-radar');
    expect(root.getElementById('on-screen-btn')).toBeNull();
    expect(cornersOf(root).map((p) => p.corner[0]).sort())
      .toEqual(['bottom-left', 'top-left', 'top-right']);
  });

  it('keeps the shadow-DOM button id distinct from the light-DOM chart one', () => {
    // `console-core.js` resolves the Navigation adapter's `supportsChart` from
    // `#btn-on-screen`, a LIGHT-DOM id that only gui/battleship/navigation.html
    // writes. The scope's own button is `#on-screen-btn` inside a shadow root.
    // Collapsing the two ids would make every console with a scope claim to
    // support a chart it does not have.
    const root = mount('ph-helm-radar');
    expect(root.getElementById('on-screen-btn')).not.toBeNull();
    expect(root.getElementById('btn-on-screen')).toBeNull();

    const core = fs.readFileSync(path.join(GUI, 'console-core.js'), 'utf8');
    expect(core).toContain("getElementById('btn-on-screen')");
    const navigation = fs.readFileSync(path.join(GUI, 'battleship', 'navigation.html'), 'utf8');
    expect(navigation).toContain('id="btn-on-screen"');
  });

  it('declares the ON SCREEN button once, not once per scope that shows it', () => {
    // The corner offsets and the button's own rules live in ph-scope-chrome.js.
    // A component that writes either for itself again is the duplication this
    // slice removed, and it is invisible until the two copies disagree.
    for (const file of ['ph-helm-radar.js', 'ph-sensor-radar.js']) {
      const source = readStripped(path.join(GUI, 'components', file));
      expect(source, file + ' writes its own ON SCREEN rules')
        .not.toMatch(/\.on-screen-btn\s*\{/);
      expect(source, file + ' writes its own ON SCREEN markup')
        .not.toMatch(/<button/);
    }
  });
});

describe('the scale readout stays on the ring, clear of all four corners', () => {
  /** The scale readout's paint anchor for a 500-unit scope. */
  function scaleAnchor() {
    const ctx = scope().draw({ range: 500, blips: [] });
    const hits = paintedText(ctx)
      .filter((p) => p.text === t('console.radar.scale', { range: '500' }));
    expect(hits.length).toBe(1);
    return hits[0];
  }

  it('paints it against the outer ring rather than out in a corner', () => {
    const readout = scaleAnchor();
    // Just inside the outermost stroked radius — on the ring it names, not out
    // at the corner where a corner label would be.
    const radius = Math.hypot(readout.x - R, readout.y - R);
    expect(radius).toBeLessThan(R);
    expect(radius).toBeGreaterThan(R * 0.9);
  });

  it('keeps out of the corner ON SCREEN occupies', () => {
    // It used to be painted on the aft-STARBOARD diagonal — the bottom-right —
    // which is the corner the fourth slot's button now stands in. That button
    // is opaque enough to hide the readout completely, so the scale moved to
    // the aft-port diagonal, sharing its corner with the shortest of the three
    // readouts instead.
    const readout = scaleAnchor();
    expect(readout.x).toBeLessThan(R);       // port half
    expect(readout.y).toBeGreaterThan(R);    // aft half
  });

  it('clears the speed readout in that corner at every scope size', () => {
    // The two are anchored differently — the corner label at a PERCENTAGE of
    // the scope's side, the scale at a fixed offset from the ring — so they
    // close on each other as the scope shrinks. The gap is
    // 0.0864·side − lineHeight/px, which stays positive for any scope a phone
    // would render; this pins the arithmetic at the anchor the draw log shows.
    const readout = scaleAnchor();
    const sideCss = CSS_SIZE;
    const speedLabelTopCss = sideCss * 0.94 - LINE_HEIGHT / PX;
    expect(readout.y / PX).toBeLessThan(speedLabelTopCss);
  });
});

describe('one rule makes every scope square', () => {
  const CONSOLE_CSS = path.join(GUI, 'console.css');
  const SCOPES = 'ph-helm-radar, ph-tactical-radar, ph-sensor-radar, ph-courier-radar';

  it('states the square as min(free width, free height) in gui/console.css', () => {
    const rules = cssRules(readStripped(CONSOLE_CSS));
    const cell = rules.find((r) => r.selector === '.scope-cell > *');
    expect(cell, 'gui/console.css declares no .scope-cell child rule').toBeDefined();
    // The side is the SMALLER of the two axes of the slot the layout left over,
    // written as that sentence rather than as a min/max clamp to unpick.
    expect(cell.body.replace(/\s+/g, ' ')).toMatch(/width:\s*min\(100cqw,\s*100cqh\)/);
    const slot = rules.find((r) => r.selector === '.scope-cell');
    expect(slot.body).toMatch(/container-type:\s*size/);
  });

  it('mounts every scope in the fleet inside a .scope-cell', () => {
    const parser = new DOMParser();
    const mounted = [];
    for (const file of consoleDocuments()) {
      const doc = parser.parseFromString(fs.readFileSync(file, 'utf8'), 'text/html');
      for (const el of doc.querySelectorAll(SCOPES)) {
        const tag = el.tagName.toLowerCase();
        mounted.push(rel(file) + ' :: ' + tag);
        expect(
          !!el.parentElement && el.parentElement.classList.contains('scope-cell'),
          rel(file) + ' mounts <' + tag + '> outside a .scope-cell',
        ).toBe(true);
      }
    }
    // The sweep is worthless if it found nothing to sweep.
    expect(mounted.length).toBeGreaterThanOrEqual(10);
  });

  it('leaves no console document declaring a scope aspect-ratio of its own', () => {
    // Eight documents each worked out their own `aspect-ratio: 1 / 1` variant,
    // and each carried a comment explaining why THAT console's arithmetic had
    // to change when the chrome above the scope did. One rule, or eight
    // chances for a hull's scope to stop being square.
    const strays = [];
    for (const file of consoleDocuments()) {
      for (const rule of cssRules(readStripped(file))) {
        if (!/aspect-ratio/.test(rule.body)) continue;
        strays.push(rel(file) + ' :: ' + rule.selector);
      }
    }
    expect(strays).toEqual([]);
  });

  it('names the two side-column widths instead of spelling out a track list', () => {
    // `--scope-rail-start` / `--scope-rail-end` are the only thing a console
    // says about the row its scope sits in; the row itself is written once.
    const rules = cssRules(readStripped(CONSOLE_CSS));
    const row = rules.find((r) => r.selector === '.scope-row');
    expect(row).toBeDefined();
    expect(row.body).toMatch(/--scope-rail-start:/);
    expect(row.body).toMatch(/--scope-rail-end:/);

    const railed = [];
    for (const file of consoleDocuments()) {
      const source = readStripped(file);
      if (!/--scope-rail-(start|end)\s*:/.test(source)) continue;
      railed.push(rel(file));
      expect(source, rel(file) + ' still writes its own scope track list')
        .not.toMatch(/grid-template-columns/);
    }
    expect(railed.length).toBeGreaterThanOrEqual(5);
  });

  it('gives the scope cell a shrink weight so a narrow row does not land on it', () => {
    // The bug this pins is a `flex: 1 1 0` on the cell. Flex shrinking is
    // weighted by BASE SIZE, so a zero base size is a zero shrink weight: the
    // cell then takes only the positive free space left over after every rail
    // has claimed its full basis, and reaches 0px before a rail gives up a
    // pixel. `auto` makes the base size the square the row's height gives it,
    // which is what puts the cell and the rails in the same distribution.
    //
    // Text, not layout: jsdom has no layout engine. The result this produces
    // is measured in a browser by tests/smoke/scope-responsive.spec.js, which
    // is the half of the contract that catches a 66x66 scope.
    const rules = cssRules(readStripped(CONSOLE_CSS));
    const cell = rules.find((r) => r.selector === '.scope-row > .scope-cell');
    expect(cell, 'gui/console.css declares no landscape .scope-cell flex').toBeDefined();
    expect(cell.body.replace(/\s+/g, ' ')).toMatch(/flex:\s*1\s+1\s+auto/);
    expect(cell.body, 'a zero flex basis takes the cell out of the shrink distribution')
      .not.toMatch(/flex:\s*1\s+1\s+0/);
  });

  it('lets a rail holding a sized widget declare the floor it shrinks to', () => {
    // The other half of sharing the shortfall: a rail that shrinks past its
    // content pushes that content out SIDEWAYS over the scope, because a
    // console column does not clip. `--scope-rail-min` is that floor, 0 by
    // default so a rail of plain readouts keeps shrinking, and it inherits so
    // a console with two rails can raise it on the one that needs more.
    const rules = cssRules(readStripped(CONSOLE_CSS));
    const row = rules.find((r) => r.selector === '.scope-row');
    expect(row.body).toMatch(/--scope-rail-min:\s*0px/);
    for (const selector of [
      '.scope-row > :not(.scope-cell):first-child',
      '.scope-row > .scope-cell ~ :not(.scope-cell)',
    ]) {
      const rail = rules.find((r) => r.selector === selector);
      expect(rail, 'gui/console.css declares no rule for ' + selector).toBeDefined();
      expect(rail.body, selector + ' has no floor to stop shrinking at')
        .toMatch(/min-width:\s*var\(--scope-rail-min\)/);
    }
  });

  it('keeps the Helm joystick widgets sized to their rail rather than over it', () => {
    // The Destroyer's Helm is the one console with a scope BETWEEN two control
    // rails, and both rails hold a widget with a size of its own. Those sizes
    // are maxima — `min(<size>, 100%)` — so a rail at its floor renders a
    // smaller control; a bare `width: <size>` renders the same control hanging
    // over the scope, which no size assertion can see.
    for (const [file, size] of [
      ['components/ph-helm-joystick.js', '240px'],
      ['components/ph-lateral-thrust-joystick.js', '180px'],
    ]) {
      const source = fs.readFileSync(path.join(GUI, file), 'utf8');
      expect(source, file + ' pins its widget to a fixed width')
        .toMatch(new RegExp('width:\\s*min\\(' + size + ',\\s*100%\\)'));
    }
  });
});
