// @vitest-environment jsdom
/**
 * tests/client/tactical-sensors-contact-cues.test.js — issue #1424 (PRD
 * #1418 stories 6, 7; the parent's "hostile/selected/locked distinctions
 * with non-colour cues" and "forced-colour and high-contrast states
 * meaningful" acceptance criteria for Tactical + Sensors).
 *
 * Three gaps this pins, all in the shared `ph-radar.js` scope every Tactical
 * and Sensors console mounts through `ph-tactical-radar.js` / `ph-sensor-
 * radar.js`:
 *
 *  1. HOSTILE had no cue at all once a contact's icon image loaded — the
 *     colour tint (`b.color`) only ever reached the FALLBACK filled circle an
 *     unloaded icon takes for one frame; `octx.drawImage(icon, …)` painted no
 *     tint whatsoever. A hostile heavy and a friendly one of the same class
 *     were pixel-identical.
 *  2. SELECTED vs LOCKED distinguished themselves by colour and a 2px radius
 *     offset alone (`--signal` +6px vs `--fire-hot` +8px) — a colour-only
 *     distinction PRD #1418 explicitly asks not to rely on.
 *  3. FORCED COLOURS never reached the canvas at all: `phColor()` resolves
 *     `var(--x)` against a COMPUTED custom-property value, and custom
 *     properties are exempt from the browser's forced-colours override (see
 *     gui/tokens.css's own comment on this) — so every ring/marker kept its
 *     authored hex regardless of the user's forced palette.
 *
 * Uses the same recording-context harness as radar-scope-contract.test.js
 * (module 2's draw contract, PRD #1023) — images deliberately never "load",
 * so a regular contact takes the fallback fill path and the marker/ring
 * assertions read the SAME draw log radar-scope-contract.test.js already
 * trusts (`strokedArcRadii`), rather than a second hand-rolled stub.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import '../../gui/components/ph-radar.js';
import { makeRadarCtx, strokedArcRadii } from './radar-canvas-stub.js';

const CSS_SIZE = 300;
const DPR = 2;
const PX = 2; // buffer pixels per CSS pixel

let fakeCtx;
let rafCb;
let origGetContext;
let origRAF;
let origCARAF;
let origRO;
let origImage;
let origMatchMedia;

beforeEach(() => {
  fakeCtx = makeRadarCtx();
  origGetContext = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = function () { return fakeCtx; };
  origRAF = window.requestAnimationFrame;
  window.requestAnimationFrame = vi.fn((cb) => { rafCb = cb; return 1; });
  origCARAF = window.cancelAnimationFrame;
  window.cancelAnimationFrame = vi.fn();
  origRO = window.ResizeObserver;
  window.ResizeObserver = function () {
    return { observe: vi.fn(), disconnect: vi.fn() };
  };
  origImage = window.Image;
  // Never "loaded" (as radar-scope-contract.test.js does): a regular contact
  // takes the fallback filled-circle path, which is the path the hostile
  // marker sits directly after.
  window.Image = class {
    constructor() { this.naturalWidth = 0; this.complete = false; }
    set src(v) { this._src = v; }
    get src() { return this._src; }
  };
  origMatchMedia = window.matchMedia;
  delete window.matchMedia;
  Object.defineProperty(window, 'devicePixelRatio', { value: DPR, configurable: true });
});

afterEach(() => {
  HTMLCanvasElement.prototype.getContext = origGetContext;
  window.requestAnimationFrame = origRAF;
  window.cancelAnimationFrame = origCARAF;
  window.ResizeObserver = origRO;
  window.Image = origImage;
  if (origMatchMedia) window.matchMedia = origMatchMedia; else delete window.matchMedia;
  document.body.innerHTML = '';
});

/** Mock `window.matchMedia('(forced-colors: active)')` to report `matches`. */
function stubForcedColors(matches) {
  window.matchMedia = vi.fn((query) => ({ matches: query === '(forced-colors: active)' && matches }));
}

/** A mounted `<ph-radar>`, sized as radar-scope-contract.test.js's does. */
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

/** A single contact at the scope centre; `scaled_radius: 0` pins dotR to the
 *  6px-CSS floor (`6 * PX` buffer px) so ring/marker geometry is exact. */
function contact(extra) {
  return { uuid: 'c1', radar_x: 0, radar_y: 0, scaled_radius: 0, kind: 'ship', icon: 'ship', ...extra };
}

// ── 1. Hostile marker: a shape, present only for a real IFF-hostile blip ───

describe('the hostile contact marker', () => {
  it('is drawn for a contact whose target_tags include hostile', () => {
    const s = scope();
    const ctx = s.draw({ range: 0, blips: [contact({ target_tags: ['hostile'] })] });
    // A triangle: moveTo, two lineTo, closePath, then fill (not stroke) — the
    // shape this test exists to pin, distinct from every ring below (which
    // strokes an arc) and from the fallback circle (which fills an ARC, not a
    // path of lineTo calls).
    const triangleFills = ctx._log.filter((e, i) => e.name === 'fill'
      && ctx._log[i - 1] && ctx._log[i - 1].name === 'closePath');
    expect(triangleFills.length).toBe(1);
  });

  it('is case-insensitive about the tag', () => {
    const s = scope();
    for (const tag of ['HOSTILE', 'Hostile', 'hostile']) {
      const ctx = s.draw({ range: 0, blips: [contact({ target_tags: [tag] })] });
      const triangleFills = ctx._log.filter((e, i) => e.name === 'fill'
        && ctx._log[i - 1] && ctx._log[i - 1].name === 'closePath');
      expect(triangleFills.length, `tag "${tag}"`).toBe(1);
    }
  });

  it('is absent for a friendly or neutral contact, and for one with no tags at all', () => {
    const s = scope();
    for (const tags of [['friendly'], ['neutral', 'civilian'], [], undefined]) {
      const ctx = s.draw({ range: 0, blips: [contact({ target_tags: tags })] });
      const triangleFills = ctx._log.filter((e, i) => e.name === 'fill'
        && ctx._log[i - 1] && ctx._log[i - 1].name === 'closePath');
      expect(triangleFills.length, `tags ${JSON.stringify(tags)}`).toBe(0);
    }
  });

  it('never throws on a malformed target_tags value', () => {
    const s = scope();
    expect(() => s.draw({ range: 0, blips: [contact({ target_tags: 'hostile' })] })).not.toThrow();
    expect(() => s.draw({ range: 0, blips: [contact({ target_tags: null })] })).not.toThrow();
  });

  it('paints the authored fire-hot token normally, and the forced-colours Mark keyword when the browser is in forced-colours mode', () => {
    const s = scope();
    stubForcedColors(false);
    const normal = s.draw({ range: 0, blips: [contact({ target_tags: ['hostile'] })] });
    expect(normal.fillStyle).toBe('var(--fire-hot)');

    stubForcedColors(true);
    const forced = s.draw({ range: 0, blips: [contact({ target_tags: ['hostile'] })] });
    expect(forced.fillStyle).toBe('Mark');
  });
});

// ── 2. Selected vs locked: a shape difference, not only a colour one ───────

describe('selected and locked contact rings', () => {
  // Range rings draw first, every frame, whether or not `range` is set (the
  // harvested 0.33/0.66/1.0 fallback — see #drawRangeRings) — so a contact
  // ring assertion reads the TAIL of the stroked-arc log, never the whole
  // thing, the same way range-ring assertions elsewhere in this suite read
  // the head of it.

  it('draws the selected candidate as one single ring', () => {
    const s = scope();
    const ctx = s.draw({
      range: 0, blips: [contact()], selected_target_uuid: 'c1', target_uuid: null,
    });
    // dotR floors at 6*PX=12; the selected ring is dotR + 6*PX = 24.
    expect(strokedArcRadii(ctx).slice(-1)).toEqual([24]);
    expect(ctx.strokeStyle).toBe('var(--signal)');
  });

  it('draws a Tactical lock as two concentric rings, distinct in RADIUS from the selected ring', () => {
    const s = scope();
    const ctx = s.draw({
      range: 0, blips: [contact()], selected_target_uuid: null, target_uuid: 'c1',
    });
    // dotR=12; the lock draws at dotR+8*PX=28 and a second ring further out.
    const radii = strokedArcRadii(ctx).slice(-2);
    expect(radii[0]).toBe(28);
    expect(radii[1]).toBeGreaterThan(radii[0]);
    expect(ctx.strokeStyle).toBe('var(--fire-hot)');
  });

  it('draws BOTH rings when a Tactical lock is also the selected candidate (the real Tactical wire shape)', () => {
    // gui/stations/tactical-console.js sets selected_target_uuid AND
    // target_uuid to the SAME uuid — see makeTacticalRender's radar block —
    // so a locked Tactical target shows the single selected ring nested
    // inside the double lock ring: three strokes, two distinct shapes.
    const s = scope();
    const ctx = s.draw({
      range: 0, blips: [contact()], selected_target_uuid: 'c1', target_uuid: 'c1',
    });
    expect(strokedArcRadii(ctx).slice(-3)).toEqual([24, 28, 36]);
  });

  it('draws only the single selected ring for a Sensors selection (ph-sensor-radar never sets target_uuid)', () => {
    // Mirrors gui/components/ph-sensor-radar.js's own state contract: only
    // selected_target_uuid is ever set. Pinned here at the ph-radar.js level
    // — ph-sensor-radar.test.js already pins the wrapper never sets the other
    // field; this proves what that field's ABSENCE actually draws.
    const s = scope();
    const ctx = s.draw({
      range: 0, blips: [contact()], selected_target_uuid: 'c1',
    });
    expect(strokedArcRadii(ctx).slice(-1)).toEqual([24]);
  });

  it('respects forced colours for both rings, keeping them distinguishable system keywords', () => {
    // `s.draw()` hands back the SAME recording context every call (only its
    // log is reset) — so each colour is read out as a primitive immediately,
    // never held as a reference to compare after a later draw has mutated it.
    const s = scope();
    stubForcedColors(true);
    const selectedColor = s.draw({
      range: 0, blips: [contact()], selected_target_uuid: 'c1', target_uuid: null,
    }).strokeStyle;
    expect(selectedColor).toBe('Highlight');

    const lockedColor = s.draw({
      range: 0, blips: [contact({ uuid: 'c2' })], selected_target_uuid: null, target_uuid: 'c2',
    }).strokeStyle;
    expect(lockedColor).toBe('Mark');
    expect(lockedColor).not.toBe(selectedColor);
  });
});
