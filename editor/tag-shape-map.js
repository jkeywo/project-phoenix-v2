/**
 * tag-shape-map.js — Pure icon→RadarShape mapping for the editor preview.
 *
 * The editor draws a simplified vector glyph (Triangle/Diamond/Ring/Dot)
 * instead of loading the real radar icon PNGs the live client uses. This
 * module maps the entity's own authored `[radar_appearance].icon` string to
 * a glyph — it is keyed by icon name, never by entity tags. An entity with
 * no `[radar_appearance]` (or an icon string this map doesn't recognise)
 * still gets a sensible glyph; the absence of *any* radar_appearance is a
 * separate fallback handled upstream (see `RADAR_SHAPE_FALLBACK` in
 * canvas-world.js), not by this function.
 */

export const RADAR_SHAPE = Object.freeze({
  Triangle: 'Triangle',
  Square: 'Square',
  Diamond: 'Diamond',
  Ring: 'Ring',
  Dot: 'Dot',
});

/** Icon-name substrings that map to a glyph other than the Dot default. */
const SHIP_LIKE = ['ship', 'battleship', 'cruiser', 'destroyer'];
const STATION_LIKE = ['station'];
const PLANET_LIKE = ['planet'];

/**
 * Return the RadarShape glyph for a given icon name.
 *
 * @param {string|null|undefined} icon
 * @returns {'Triangle'|'Square'|'Diamond'|'Ring'|'Dot'}
 */
export function iconShape(icon) {
  if (!icon) return RADAR_SHAPE.Dot;
  const value = String(icon).toLowerCase();

  if (SHIP_LIKE.some((s) => value.includes(s))) return RADAR_SHAPE.Triangle;
  if (STATION_LIKE.some((s) => value.includes(s))) return RADAR_SHAPE.Diamond;
  if (PLANET_LIKE.some((s) => value.includes(s))) return RADAR_SHAPE.Ring;

  return RADAR_SHAPE.Dot;
}
