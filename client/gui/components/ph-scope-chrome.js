// gui/components/ph-scope-chrome.js — the parts of a scope that are not the scope.
//
// Four components wrap <ph-radar>: ph-tactical-radar, ph-helm-radar,
// ph-sensor-radar, and ph-courier-radar (which extends the tactical one). Three
// of them carried a byte-identical copy of the same corner-label block — the
// CSS, the three <div>s, and the three lines of JS that fill them — because a
// shadow root inherits custom properties but not markup, and the obvious way to
// give a second component the same chrome is to paste it.
//
// The cost was not the duplication itself. It was that the three copies had
// already begun to disagree about WHERE THE NUMBERS COME FROM: helm and
// tactical read `s.x` / `s.z` / `s.speed`, sensors reads `val.ship_x` /
// `val.ship_z` / `val.ship_speed`. Same three readouts, three field names, and
// nothing anywhere that said the two shapes were meant to be the same thing.
//
// So the fragment below takes the READINGS, not the state object: a caller
// unpacks its own payload's field names once and hands over `{x, z, headingDeg,
// speed}`. The divergence stays where it belongs — at the wire boundary — and
// the rendering is written once.
//
// ── Why the arc cap lives here too ─────────────────────────────────────────
//
// It is scope chrome by the same definition: an overlay drawn over the scope
// that must not stop the scope being readable. See `applyArcCompositeCap`.
import { t } from '../strings.js';
import { phColor } from './ph-console-styles.js';

/**
 * The corner-label rules, written once.
 *
 * Sizes come from the type ramp (`--text-xs` is a `max()` against the absolute
 * floor), so a narrow phone cannot shrink these below legible — which is what
 * a bare `0.65rem` against the console's `clamp(11px, 3vw, 15px)` root did.
 */
export const SCOPE_CHROME_CSS = [
  '.corner-label {',
  '  position: absolute; pointer-events: none; z-index: 10;',
  '  font-family: var(--font-mono); font-size: var(--text-xs);',
  '  letter-spacing: var(--tracking); color: var(--edge-strong);',
  '}',
  '.corner-label.top-left { top: 4%; left: 6%; }',
  '.corner-label.top-right { top: 4%; right: 6%; text-align: right; }',
  '.corner-label.bottom-left { bottom: 6%; left: 6%; }',
].join('\n');

/**
 * The three corner readouts as markup, with the ids `updateScopeChrome` writes
 * into. Indented to sit inside a container element.
 */
export function scopeChromeMarkup(indent = '  ') {
  return [
    indent + '<div class="corner-label top-left" id="label-pos"></div>',
    indent + '<div class="corner-label top-right" id="label-bearing"></div>',
    indent + '<div class="corner-label bottom-left" id="label-speed"></div>',
  ].join('\n');
}

/**
 * Fill the three corner readouts.
 *
 * @param {ShadowRoot|Element} root      the scope's shadow root
 * @param {object} readings
 * @param {number} readings.x           ship world X
 * @param {number} readings.z           ship world Z
 * @param {number} readings.headingDeg  ship heading, degrees
 * @param {number} readings.speed       forward speed, world units per second
 */
export function updateScopeChrome(root, readings) {
  if (!root) return;
  const r = readings || {};

  const posLabel = root.getElementById('label-pos');
  if (posLabel) {
    const x = Number.isFinite(r.x) ? r.x : 0;
    const z = Number.isFinite(r.z) ? r.z : 0;
    posLabel.textContent = t('console.common.radar_pos', { x: x.toFixed(0), z: z.toFixed(0) });
  }

  const bearingLabel = root.getElementById('label-bearing');
  if (bearingLabel) {
    const h = Number.isFinite(r.headingDeg) ? ((r.headingDeg % 360) + 360) % 360 : 0;
    bearingLabel.textContent = String(h.toFixed(0)).padStart(3, '0') + '°';
  }

  const speedLabel = root.getElementById('label-speed');
  if (speedLabel) {
    const spd = Number.isFinite(r.speed) ? r.speed : 0;
    speedLabel.textContent = (spd * 3.6).toFixed(1) + ' km/s';
  }
}

/**
 * The most opaque a stack of firing-arc overlays may ever composite to.
 *
 * A contact inside your own arcs still has to be visible; that is the whole
 * reason the arcs are translucent.
 */
export const ARC_COMPOSITE_MAX = 0.5;

/**
 * Cap the COMPOSITE alpha of an arc group, not the alpha of each arc in it.
 *
 * Translucent fills do not average, they accumulate: n overlays at alpha `a`
 * composite to `1 - (1 - a)^n`. Four phaser banks at the authored 0.3 reach
 * 0.76 where they overlap — dark enough to swallow the contact the officer is
 * about to shoot — and eight reach 0.94. Capping each ARC instead just moves
 * the number the stack starts from; the stack still climbs to 1.
 *
 * SVG group opacity is applied to the FLATTENED group, so a `<g opacity="c">`
 * multiplies whatever its children composited to by `c` exactly once, however
 * many children there are. That is the cap, expressed structurally rather than
 * arithmetically:
 *
 *   composite = (1 - Π(1 - aᵢ)) × c  ≤  c
 *
 * The children's alphas are then divided through by `c`, so a LONE arc renders
 * at exactly the alpha its author asked for — `(0.3 / 0.5) × 0.5 = 0.3` — and
 * only the stack is pulled back. Relative weighting between arcs survives; a
 * bank the server marked fainter stays fainter.
 *
 * @param {SVGGElement|null} group  the <g> holding the arc paths
 * @param {number} cap              the composite ceiling
 */
export function applyArcCompositeCap(group, cap = ARC_COMPOSITE_MAX) {
  if (!group) return;
  group.setAttribute('opacity', String(cap));
}

/**
 * The per-arc alpha to write on a child of a capped group so that one arc alone
 * still renders at `authored`.
 *
 * Clamped at 1: an arc authored more opaque than the cap renders AT the cap,
 * which is the cap doing its job rather than a rounding accident.
 */
export function cappedArcAlpha(authored, cap = ARC_COMPOSITE_MAX) {
  const a = Number.isFinite(authored) ? authored : 0;
  if (!(cap > 0)) return 0;
  return Math.min(1, Math.max(0, a / cap));
}

/**
 * Resolve a length token to CSS pixels against a live element.
 *
 * The sibling of `phColor`, and it exists for the same reason: a `<canvas>`
 * font string and an SVG `font-size` attribute are not CSS declarations, so
 * neither substitutes `var(--text-min)`. Rather than let the scope keep its own
 * copy of the type floor as a bare `11`, it names the token and resolves it
 * here — so raising the floor in gui/tokens.css raises it on the scope too.
 *
 * @param {Element} el
 * @param {string} name       a custom property holding an absolute length
 * @param {number} fallback   px to use where nothing can resolve it (Node, jsdom)
 * @returns {number}          the resolved length in CSS pixels
 */
export function phPx(el, name, fallback) {
  if (!el || typeof getComputedStyle !== 'function') return fallback;
  let style;
  try { style = getComputedStyle(el); } catch (_) { return fallback; }
  if (!style || typeof style.getPropertyValue !== 'function') return fallback;
  const raw = (style.getPropertyValue(name) || '').trim();
  const px = parseFloat(raw);
  return Number.isFinite(px) && px > 0 ? px : fallback;
}

/** The absolute type floor, used wherever `--text-min` cannot be resolved. */
export const TEXT_MIN_FALLBACK_PX = 11;

/**
 * Pick a "nice" spacing for range rings: 1, 2 or 5 times a power of ten.
 *
 * The dormant gui/radar-widget.js drew three rings at a fixed 33 / 66 / 100 %
 * of the scope radius. They looked like rings, but they measured nothing — the
 * middle ring's radius was whatever two-thirds of that console's range happened
 * to be, so a 500-unit helm scope and a 300-unit weapons scope drew identical
 * pictures of different distances. A ring is only worth drawing if a player can
 * read a number off it.
 *
 * So the spacing is chosen from the ladder a chart would use, aiming for about
 * five rings: 500 → 100, 300 → 100, 1200 → 200, 80 → 20.
 *
 * @param {number} range     the scope's outer radius in world units
 * @param {number} target    roughly how many rings to draw
 * @returns {number}         the ring spacing, or 0 when `range` is unusable
 */
export function ringStep(range, target = 5) {
  if (!Number.isFinite(range) || range <= 0 || !(target > 0)) return 0;
  const raw = range / target;
  const magnitude = 10 ** Math.floor(Math.log10(raw));
  for (const multiple of [1, 2, 5, 10]) {
    const step = multiple * magnitude;
    if (step >= raw) return step;
  }
  return magnitude * 10;
}

/**
 * The radii to draw rings at, as fractions of the scope radius, paired with the
 * world distance each one stands for.
 *
 * The outermost ring is always the scope edge itself, whatever the step —
 * without it the scope has no boundary and a contact clamped to the rim looks
 * like it is floating outside the picture.
 *
 * @param {number} range
 * @returns {{fraction: number, distance: number}[]}
 */
export function ringPlan(range) {
  const step = ringStep(range);
  if (step <= 0) return [];
  const out = [];
  for (let d = step; d < range - step * 0.05; d += step) {
    out.push({ fraction: d / range, distance: d });
  }
  out.push({ fraction: 1, distance: range });
  return out;
}

/** The scale readout drawn against the outermost ring. */
export function scaleReadout(range) {
  if (!Number.isFinite(range) || range <= 0) return '';
  return t('console.radar.scale', { range: range.toFixed(0) });
}

/** Re-exported so a scope needs one import for its token-resolving helpers. */
export { phColor };
