/**
 * tag-shape-map.js — Pure tag→RadarShape mapping.
 *
 * Mirrors the runtime logic in src/console/helm/client.rs:
 *   has_tag("ship")    → RadarShape::Triangle
 *   has_tag("station") → RadarShape::Square
 *   (everything else)  → RadarShape::Dot
 *
 * Order of precedence matches the Rust runtime: ship is checked first,
 * then station, then fallback to Dot.
 */

export const RADAR_SHAPE = Object.freeze({
  Triangle: 'Triangle',
  Square: 'Square',
  Dot: 'Dot',
});

/**
 * Return the RadarShape for a given tag array.
 *
 * @param {string[]|null|undefined} tags
 * @returns {'Triangle'|'Square'|'Dot'}
 */
export function tagShape(tags) {
  if (!Array.isArray(tags)) return RADAR_SHAPE.Dot;

  const has = (tag) => tags.includes(tag);

  if (has('ship')) return RADAR_SHAPE.Triangle;
  if (has('station')) return RADAR_SHAPE.Square;
  return RADAR_SHAPE.Dot;
}
