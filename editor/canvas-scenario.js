/**
 * canvas-scenario.js — Pure logic for scenario-mode canvas rendering.
 *
 * Decouples appearance resolution from Konva/DOM so it is unit-testable.
 */

import { tagShape } from './tag-shape-map.js';

/** Sentinel value returned as `shape` when no radar_appearance is present. */
export const RADAR_SHAPE_FALLBACK = 'X';

/**
 * Resolve the visual appearance of a world entity for canvas rendering.
 *
 * @param {object} entity    - Parsed TOML entity object (may have radar_appearance).
 * @param {object} [anchors] - The `worldState.anchors` flat TOML map `{ anchorName: [x,y,z] }`.
 * @returns {{
 *   colour: [number, number, number],
 *   radius: number,
 *   shape: string,
 *   hasFallback: boolean,
 *   x: number,
 *   z: number,
 * }}
 */
export function resolveEntityAppearance(entity, anchors = {}) {
  const tags = entity.tags || [];
  const shape = tagShape(tags);

  const radarApp = entity.radar_appearance;
  const hasFallback = !radarApp;

  const colour = radarApp
    ? radarApp.colour
    : [0.7, 0.7, 0.7]; // neutral grey for X fallback

  const radius = radarApp
    ? radarApp.radius
    : 8.0; // neutral fallback radius

  // Resolve position
  let x = 0;
  let z = 0;

  if (entity.position && Array.isArray(entity.position)) {
    x = entity.position[0];
    z = entity.position[2];
  } else if (entity.anchor && anchors && typeof anchors === 'object') {
    const pos = anchors[entity.anchor];
    if (Array.isArray(pos) && pos.length >= 3) {
      x = pos[0];
      z = pos[2];
    }
  }

  return { colour, radius, shape, hasFallback, x, z };
}

/**
 * Convert a normalised [r, g, b] colour array to a CSS hex string.
 *
 * @param {[number, number, number]} colour
 * @returns {string} e.g. "#ff3333"
 */
export function colourToHex(colour) {
  const [r, g, b] = colour;
  const toHex = (v) => Math.round(Math.min(1, Math.max(0, v)) * 255)
    .toString(16)
    .padStart(2, '0');
  return `#${toHex(r)}${toHex(g)}${toHex(b)}`;
}

/**
 * Draw a RadarShape on a Konva canvas context.
 * For `hasFallback` entities, draws an X in a neutral colour.
 *
 * @param {object}   group      - Konva.Group to add shapes to (must support .add())
 * @param {object}   Konva      - Konva namespace (passed in to keep module testable)
 * @param {object}   appearance - Result from resolveEntityAppearance()
 * @param {boolean}  selected   - Whether entity is currently selected
 */
export function drawEntityShape(group, Konva, appearance, selected = false) {
  const { colour, radius, shape, hasFallback } = appearance;
  const hexColour = colourToHex(colour);
  const strokeColour = selected ? '#00ff00' : '#ffffff';
  const strokeWidth = selected ? 3 : 1.5;

  if (hasFallback) {
    // X fallback: two crossed lines
    const xSize = Math.max(6, Math.min(24, radius * 0.4));
    const lineA = new Konva.Line({
      points: [-xSize, -xSize, xSize, xSize],
      stroke: '#aaaaaa',
      strokeWidth: 2,
    });
    const lineB = new Konva.Line({
      points: [xSize, -xSize, -xSize, xSize],
      stroke: '#aaaaaa',
      strokeWidth: 2,
    });
    group.add(lineA);
    group.add(lineB);
    return;
  }

  switch (shape) {
    case 'Triangle': {
      // Equilateral triangle pointing up
      const h = radius * 1.2;
      const w = h * 0.866; // cos(30°) ≈ 0.866
      const triangle = new Konva.Line({
        points: [0, -h, -w, h * 0.5, w, h * 0.5],
        closed: true,
        fill: hexColour + '88',
        stroke: strokeColour,
        strokeWidth,
      });
      group.add(triangle);
      break;
    }
    case 'Square': {
      const side = radius * 1.4;
      const square = new Konva.Rect({
        x: -side / 2,
        y: -side / 2,
        width: side,
        height: side,
        fill: hexColour + '88',
        stroke: strokeColour,
        strokeWidth,
      });
      group.add(square);
      break;
    }
    default: {
      // Dot
      const circle = new Konva.Circle({
        radius: Math.max(3, radius * 0.15),
        fill: hexColour,
        stroke: strokeColour,
        strokeWidth,
      });
      group.add(circle);
      break;
    }
  }
}
