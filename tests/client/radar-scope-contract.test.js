// @vitest-environment jsdom
/**
 * tests/client/radar-scope-contract.test.js — module 2's draw contract (PRD #1023).
 *
 * The PRD asks for three things of the hardened scope, and names the test for
 * each: "rings present at the configured ranges, labels haloed and
 * non-overlapping for a clustered fixture, arc alpha capped under stacked
 * overlays". Those are the three describes below.
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
import { t } from '../../gui/strings.js';
import '../../gui/components/ph-radar.js';
import '../../gui/components/ph-tactical-radar.js';
import { makeRadarCtx, strokedArcRadii, paintedText, haloedText } from './radar-canvas-stub.js';
import { ringPlan, ringStep, ARC_COMPOSITE_MAX } from '../../gui/components/ph-scope-chrome.js';

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
 * part of the de-collision pass: it is anchored to the scope's rim rather than
 * to a contact, and it is drawn right-aligned from a bottom baseline, so the
 * box arithmetic below would not describe it correctly anyway.
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
