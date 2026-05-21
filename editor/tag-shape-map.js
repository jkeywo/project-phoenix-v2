/**
 * tag-shape-map.js — Pure tag→RadarShape mapping.
 *
 * Mirrors the runtime table in src/gui/radar.rs `tags_to_radar_layer` +
 * `layer_to_icon`. Each Rust `RadarLayer` maps to one editor `RadarShape`:
 *
 *   Rust layer  →  editor shape
 *   ──────────────────────────
 *   Ship        →  Triangle
 *   Asteroid    →  Dot
 *   Station     →  Diamond
 *   Missile     →  Dot       (rare in editor; torpedoes are runtime-only)
 *   Planet      →  Ring
 *   Star        →  Dot       (drawn as a filled circle, distinct from Ring)
 *
 * Precedence matches the runtime (first-match-wins, identical to
 * `tags_to_radar_layer` in src/gui/radar.rs):
 *
 *   has("region")                    → Dot (regions are not radar-relevant
 *                                           in-game; editor still draws them
 *                                           as a generic dot via fallback)
 *   has("ship") | has("pirate")      → Triangle
 *   has("asteroid") | "asteroid_field" → Dot
 *   has("station")                   → Diamond
 *   has("missile") | has("torpedo")  → Dot
 *   has("planet")                    → Ring
 *   has("star")                      → Dot
 *   (otherwise)                      → Dot
 *
 * When the Rust table changes, update this file AND the matching tests in
 * editor/tests/tag-shape-map.test.js.
 */

export const RADAR_SHAPE = Object.freeze({
  Triangle: 'Triangle',
  Square: 'Square',
  Diamond: 'Diamond',
  Ring: 'Ring',
  Dot: 'Dot',
});

/**
 * Return the RadarShape for a given tag array.
 *
 * Order of checks matches `tags_to_radar_layer` in src/gui/radar.rs.
 *
 * @param {string[]|null|undefined} tags
 * @returns {'Triangle'|'Square'|'Diamond'|'Ring'|'Dot'}
 */
export function tagShape(tags) {
  if (!Array.isArray(tags)) return RADAR_SHAPE.Dot;

  const has = (tag) => tags.includes(tag);

  // Region is filtered out of the runtime radar entirely; in the editor we
  // still need *some* shape so the entity is visible on the canvas, and Dot
  // is the safest generic. Region rendering proper is handled by
  // canvas-region.js, not by this mapping.
  if (has('region')) return RADAR_SHAPE.Dot;

  if (has('ship') || has('pirate')) return RADAR_SHAPE.Triangle;
  if (has('asteroid') || has('asteroid_field')) return RADAR_SHAPE.Dot;
  if (has('station')) return RADAR_SHAPE.Diamond;
  if (has('missile') || has('torpedo')) return RADAR_SHAPE.Dot;
  if (has('planet')) return RADAR_SHAPE.Ring;
  if (has('star')) return RADAR_SHAPE.Dot;

  return RADAR_SHAPE.Dot;
}
