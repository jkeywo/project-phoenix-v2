/**
 * canvas-region.js — Pure logic for rendering region entities on the world canvas.
 *
 * Decouples appearance resolution from Konva/DOM so it is unit-testable.
 */

/** Icon for each of the six region effect types. */
const EFFECT_ICONS = {
  damage_zone:     '🔥',
  slow_zone:       '🐢',
  blocks_impulse:  '🚫',
  radar_dampening: '📡',
  comms_jammed:    '📵',
  sensor_blind:    '👁️',
};

/** Default colour when the entity has no colour field. */
const DEFAULT_COLOUR = [0.6, 0.6, 0.6];

/** All recognised effect keys, in display order. */
const EFFECT_KEYS = [
  'damage_zone',
  'slow_zone',
  'blocks_impulse',
  'radar_dampening',
  'comms_jammed',
  'sensor_blind',
];

/**
 * Resolve the rendering spec for a region entity.
 *
 * @param {object} entity - Parsed TOML region entity. Expected to contain:
 *   - shape: { type: 'sphere'|'box'|'torus', ... }
 *   - colour?: [r, g, b]        — normalised 0-1 floats
 *   - position?: [x, y, z]
 *   - effects?: { damage_zone?, slow_zone?, blocks_impulse?,
 *                 radar_dampening?, comms_jammed?, sensor_blind? }
 *
 * @returns {{
 *   shape: 'circle'|'rect'|'torus',
 *   cx: number,
 *   cz: number,
 *   // circle-only:
 *   radius?: number,
 *   // rect-only:
 *   half_x?: number,
 *   half_z?: number,
 *   // torus-only:
 *   inner_radius?: number,
 *   outer_radius?: number,
 *   colour: [number, number, number],
 *   fillAlpha: 0.15,
 *   effects: string[],
 *   effectIcons: object,
 * }}
 */
export function getRegionRenderSpec(entity) {
  const shape = entity.shape || {};
  const colour = entity.colour || DEFAULT_COLOUR;
  const effects = entity.effects || {};

  // Resolve centre position from XZ plane via nested `transform.position`.
  const tPos = entity.transform && entity.transform.position;
  const cx = Array.isArray(tPos) ? tPos[0] : 0;
  const cz = Array.isArray(tPos) ? tPos[2] : 0;

  // Collect active effects
  const activeEffects = EFFECT_KEYS.filter(key => effects[key] != null);

  // Base spec shared across all shape types
  const base = {
    colour,
    fillAlpha: 0.15,
    cx,
    cz,
    effects: activeEffects,
    effectIcons: EFFECT_ICONS,
  };

  switch (shape.type) {
    case 'sphere':
      return {
        ...base,
        shape: 'circle',
        radius: shape.radius,
      };

    case 'box':
      return {
        ...base,
        shape: 'rect',
        half_x: Array.isArray(shape.half_extents) ? shape.half_extents[0] : 0,
        half_z: Array.isArray(shape.half_extents) ? shape.half_extents[2] : 0,
      };

    case 'torus':
      return {
        ...base,
        shape: 'torus',
        inner_radius: shape.inner_radius,
        outer_radius: shape.outer_radius,
      };

    default:
      // Unknown shape type — return a circle with radius 0 as safe fallback
      return {
        ...base,
        shape: 'circle',
        radius: 0,
      };
  }
}
