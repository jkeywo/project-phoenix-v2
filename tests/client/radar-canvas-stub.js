/**
 * tests/client/radar-canvas-stub.js — one 2D context stand-in for the scopes.
 *
 * Not a test file (no `.test.js`, so vitest does not collect it).
 *
 * Five radar test files each carried their own hand-written `makeFakeCtx`, and
 * every one of them listed a DIFFERENT subset of the Canvas2D surface —
 * whichever calls the component happened to make when that file was written.
 * The consequence is worse than duplication: a stub missing a method does not
 * report a missing method, it throws mid-render, and the render simply stops.
 * The tests that then fail are the ones asserting whatever came AFTER the
 * missing call, so the failure points anywhere except at the cause. PRD #1023's
 * range rings added the first `stroke()` on the common path and knocked over
 * nine assertions about labels, layering and tap targets in one go.
 *
 * So the stub is written once and covers the whole surface the scopes use. A
 * component reaching for something new gets `undefined is not a function` here
 * as well — but in exactly one place, with one file to fix.
 */
import { vi } from 'vitest';

/** Every method the radar components call on a 2D context. */
const METHODS = [
  'fillRect', 'clearRect', 'beginPath', 'closePath', 'moveTo', 'lineTo',
  'arc', 'fill', 'stroke', 'clip', 'fillText', 'strokeText', 'drawImage',
  'save', 'restore', 'translate', 'rotate', 'scale', 'setTransform',
];

/**
 * A recording 2D context.
 *
 * Every method is a `vi.fn()` — so `ctx.arc.mock.calls` works — that ALSO
 * pushes its arguments onto `ctx._calls.arc`, the shape the older radar tests
 * read. Both idioms are live in the suite and neither is wrong.
 *
 * `measureText` returns a monospace-shaped width so the label de-collision pass
 * has real geometry to work with rather than falling back to its estimate;
 * `font` is parsed for the size, so a test that changes the font changes the
 * measurement.
 */
export function makeRadarCtx() {
  const calls = {};
  /**
   * Every call in order, across methods.
   *
   * Canvas2D is a state machine, so which call came between which others is
   * often the only thing that distinguishes two drawings: `arc()` then
   * `fill()` is a blip, `arc()` then `stroke()` is a range ring, and the
   * per-method lists cannot tell them apart.
   */
  const log = [];
  const ctx = {
    fillStyle: '',
    strokeStyle: '',
    lineWidth: 1,
    lineJoin: 'miter',
    miterLimit: 10,
    globalAlpha: 1,
    globalCompositeOperation: 'source-over',
    font: '',
    textAlign: 'start',
    textBaseline: 'alphabetic',
  };
  for (const name of METHODS) {
    calls[name] = [];
    ctx[name] = vi.fn((...args) => {
      calls[name].push(args);
      log.push({ name, args });
    });
  }
  ctx._calls = calls;
  ctx._log = log;
  // Clearing `_calls` alone would leave the ordered log stale, and a test that
  // did that would read the previous frame's rings without noticing.
  ctx._reset = () => {
    for (const list of Object.values(calls)) list.length = 0;
    log.length = 0;
  };
  ctx.measureText = vi.fn((text) => {
    const size = parseFloat(ctx.font) || 10;
    return { width: String(text).length * size * 0.6 };
  });
  return ctx;
}

/**
 * The rings the scope STROKED, as radii, in draw order.
 *
 * A ring is a full-turn `arc()` whose path was then stroked rather than
 * filled — which is exactly what separates a range ring from the fallback disc
 * the scope paints when its backdrop PNG has not loaded, and from a blip. The
 * two are indistinguishable in the per-method call lists, so this walks the
 * ordered log and asks what happened to each path after it was described.
 *
 * Reading radii back out of the draw log is the whole point: it asserts what
 * the player sees, not that some private method was called.
 */
export function strokedArcRadii(ctx) {
  const out = [];
  for (let i = 0; i < ctx._log.length; i += 1) {
    const entry = ctx._log[i];
    if (entry.name !== 'arc') continue;
    const [, , radius, start = 0, end = 0] = entry.args;
    if (Math.abs(end - start) < Math.PI * 2 - 1e-6) continue;
    // Whichever of fill/stroke terminates this path first decides what it was.
    for (let j = i + 1; j < ctx._log.length; j += 1) {
      const next = ctx._log[j].name;
      if (next === 'stroke') { out.push(radius); break; }
      if (next === 'fill' || next === 'arc' || next === 'beginPath') break;
    }
  }
  return out;
}

/** Every `fillText` as `{text, x, y}`, in draw order. */
export function paintedText(ctx) {
  return ctx._calls.fillText.map(([text, x, y]) => ({ text, x, y }));
}

/** Every `strokeText` as `{text, x, y}` — the halo behind the fills above. */
export function haloedText(ctx) {
  return ctx._calls.strokeText.map(([text, x, y]) => ({ text, x, y }));
}
