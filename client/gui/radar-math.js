/**
 * gui/radar-math.js — Pure coordinate projection math for the HTML radar widget.
 *
 * All functions are pure (no side effects, no DOM/canvas dependency).
 * Extracted from RadarWidget so projection logic is testable without a
 * browser environment.  Consumed by RadarWidget (Slice 5a / issue #447)
 * and by future console HTML pages that need client-side projection.
 *
 * Coordinate conventions (match the Rust server's `project_blip()`):
 *   World space:  +X = starboard (right), -Z = forward (north) at yaw = 0
 *   Radar space:  +rx = starboard (right on radar), +rz = forward (up on radar)
 *   Canvas space: +sx = right,  +sy = down  (Y axis inverted from radar)
 */

// Minimum effective range returned by autoScaleRange when there are no entities
// or all entities are at the radar centre.
export const RADAR_MIN_RANGE = 10.0;

/**
 * Transform world-space entity coordinates to radar-relative world units.
 *
 * @param {number} entityX     World X of the entity
 * @param {number} entityZ     World Z of the entity
 * @param {number} shipX       World X of the ship (radar centre for ship_relative / world_fixed)
 * @param {number} shipZ       World Z of the ship
 * @param {number} shipYaw     Ship heading in radians.
 *        At yaw=0: forward = world −Z, right = world +X.
 * @param {string} orientation 'ship_relative' | 'world_fixed' | 'world_centred'
 *
 * @returns {{ rx: number, rz: number }}
 *        Radar-relative coords in world units.
 *        Positive rx = starboard (right on radar display).
 *        Positive rz = forward  (up   on radar display).
 */
export function worldToRadar(entityX, entityZ, shipX, shipZ, shipYaw, orientation) {
  switch (orientation) {
    case 'world_fixed': {
      // Ship-centred, north-is-up (no yaw rotation).
      // World −Z = north = forward.  Negate Z so positive rz = north = up.
      var dx_f = entityX - shipX;
      var dz_f = entityZ - shipZ;
      return { rx: dx_f, rz: -dz_f };
    }
    case 'world_centred': {
      // World origin is radar centre; no translation.
      // Negate Z so positive rz = north = up.
      return { rx: entityX, rz: -entityZ };
    }
    case 'ship_relative':
    default: {
      // Ship-centred, ship-aligned.  Matches Rust project_blip():
      //   rx = dx·cos(yaw) + dz·sin(yaw)
      //   rz = dx·sin(yaw) − dz·cos(yaw)
      var dx  = entityX - shipX;
      var dz  = entityZ - shipZ;
      var cos = Math.cos(shipYaw);
      var sin = Math.sin(shipYaw);
      return {
        rx: dx * cos + dz * sin,
        rz: dx * sin - dz * cos,
      };
    }
  }
}

/**
 * Map radar-relative world-unit coordinates to canvas pixel offsets from centre.
 *
 * @param {number} radarX       Radar-space X in world units (positive = right)
 * @param {number} radarZ       Radar-space Z in world units (positive = up/forward)
 * @param {number} range        Effective range in world units (half-width of the radar view)
 * @param {number} canvasRadius Pixel radius of the inscribed radar circle
 *
 * @returns {{ sx: number, sy: number }}
 *        Pixel offsets from canvas centre (cx, cy).
 *        `bx = cx + sx`, `by = cy + sy`.
 *        Canvas Y increases downward, so positive radarZ → negative sy (up).
 */
export function radarToScreen(radarX, radarZ, range, canvasRadius) {
  var scale = canvasRadius / range;
  return {
    sx:  radarX * scale,
    sy: -radarZ * scale,  // negate: canvas +Y is down, radar +Z is up
  };
}

/**
 * Compute the minimum effective range that contains all entity radar positions
 * with a fractional safety margin.
 *
 * @param {Array<{rx: number, rz: number}>} radarPositions
 *        Pre-projected entity positions in radar-relative world units
 *        (as returned by `worldToRadar`).
 * @param {number} [margin=0.1]
 *        Fractional padding beyond the furthest entity distance.
 *        E.g. 0.1 = 10% clearance beyond the outermost blip.
 *
 * @returns {number}
 *        Effective range in world units.
 *        Always ≥ RADAR_MIN_RANGE (10.0).
 *        Returns RADAR_MIN_RANGE when the input array is empty or all entities
 *        are at the radar centre.
 */
export function autoScaleRange(radarPositions, margin) {
  var m = (typeof margin === 'number') ? margin : 0.1;
  if (!radarPositions || radarPositions.length === 0) {
    return RADAR_MIN_RANGE;
  }
  var maxDist = 0;
  for (var i = 0; i < radarPositions.length; i++) {
    var p = radarPositions[i];
    var rx = (p.rx != null) ? p.rx : 0;
    var rz = (p.rz != null) ? p.rz : 0;
    var dist = Math.sqrt(rx * rx + rz * rz);
    if (dist > maxDist) maxDist = dist;
  }
  if (maxDist === 0) return RADAR_MIN_RANGE;
  return Math.max(RADAR_MIN_RANGE, maxDist * (1 + m));
}

/**
 * Set a circular clip path on the canvas context so subsequent draws are
 * confined to the inscribed circle.  Caller must call `ctx.save()` first
 * and `ctx.restore()` afterwards to lift the clip.
 *
 * @param {CanvasRenderingContext2D} ctx
 * @param {number} cx      Canvas centre X in pixels
 * @param {number} cy      Canvas centre Y in pixels
 * @param {number} radius  Circle radius in pixels
 */
export function clipToCircle(ctx, cx, cy, radius) {
  ctx.beginPath();
  ctx.arc(cx, cy, radius, 0, Math.PI * 2);
  ctx.clip();
}
