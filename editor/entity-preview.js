import { tagShape } from './tag-shape-map.js';
import { RADAR_SHAPE_FALLBACK } from './canvas-scenario.js';

const CONSOLE_SECTIONS = [
  'helm_console',
  'weapons_console',
  'engineering_console',
  'captain_console',
  'sensors_console',
  'shields_console',
  'navigation_console',
];

export function computeEntityPreview(entity, factionMap = new Map()) {
  if (!entity || typeof entity !== 'object') {
    return null;
  }

  const tags = Array.isArray(entity.tags) ? entity.tags : [];
  const radarApp = entity.radar_appearance;

  const radarShape = radarApp ? tagShape(tags) : RADAR_SHAPE_FALLBACK;
  const radarColour = radarApp?.colour ?? null;
  const radarRadius = radarApp?.radius ?? null;

  const collider = entity.collider;
  const colliderShape = collider?.shape ?? null;
  const colliderRadius = collider?.radius ?? 0;
  const colliderLength = collider?.length ?? 0;

  let regionShape = null;
  const shape = entity.shape;
  if (shape) {
    if (shape.type === 'sphere') {
      regionShape = { type: 'sphere', radius: shape.radius };
    } else if (shape.type === 'box') {
      regionShape = { type: 'box', halfExtents: shape.half_extents, yaw: shape.yaw ?? 0 };
    } else if (shape.type === 'torus') {
      regionShape = { type: 'torus', innerRadius: shape.inner_radius, outerRadius: shape.outer_radius };
    }
  }

  let asteroidField = null;
  const af = entity.asteroid_field;
  if (af) {
    asteroidField = { innerRadius: af.inner_radius ?? 0, outerRadius: af.outer_radius ?? 0 };
  }

  const consoles = CONSOLE_SECTIONS.filter(
    (key) => entity[key] !== undefined && entity[key] !== null,
  );

  const hullTotal = entity.hull?.hull_integrity ?? null;

  const factionUuid = entity.faction;
  const faction = factionUuid != null ? (factionMap.get(factionUuid) ?? factionUuid) : null;

  return {
    colliderShape,
    colliderRadius,
    colliderLength,
    radarShape,
    radarColour,
    radarRadius,
    regionShape,
    asteroidField,
    showForwardArrow: true,
    textOverlay: {
      tags,
      faction,
      consoles,
      hullTotal,
    },
  };
}
