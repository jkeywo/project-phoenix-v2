/**
 * entity-preview-view.js
 *
 * DOM/Konva renderer for the Entity Mode right pane. Consumes the pure
 * `computeEntityPreview` data and draws a single fixed-viewport stage with:
 *
 *   - Forward arrow at the origin (small triangle pointing -Z, drawn up).
 *   - Collider outline (Ball → circle, Capsule → pill).
 *   - Radar shape (via tag-shape-map): triangle/dot/diamond/ring/square.
 *   - Region shape (sphere → ring, box → rect, torus → donut).
 *   - Asteroid-field donut.
 *   - Top-left text overlay: tags, faction name, consoles, hull total.
 *
 * The viewport is sized to the largest dimension across all primitives,
 * with ~20% padding, and centred on (0,0). Konva is read off `window.Konva`
 * (loaded by editor.html); tests inject a mock via the second argument.
 */
import { RADAR_SHAPE } from './tag-shape-map.js';

const PADDING_RATIO = 0.2;
const MIN_VIEW = 60;
const DEFAULT_VIEW = 240;

export function renderEntityPreviewView(host, preview, { Konva } = {}) {
  if (!host) return null;
  // Clear host
  host.innerHTML = '';

  const K = Konva ?? (typeof window !== 'undefined' ? window.Konva : null);
  if (!K) {
    const msg = document.createElement('p');
    msg.className = 'entity-preview-error';
    msg.textContent = 'Konva not available — preview disabled.';
    host.appendChild(msg);
    return null;
  }

  if (!preview || preview.placeholder) {
    const ph = document.createElement('p');
    ph.className = 'placeholder';
    ph.textContent = preview?.activeFile ? 'Loading preview…' : 'No entity selected.';
    host.appendChild(ph);
    return null;
  }

  // Compute world extent (largest of collider/radar/region/field).
  const extent = computeExtent(preview);
  const viewSize = Math.max(MIN_VIEW, extent * (1 + PADDING_RATIO * 2)) || DEFAULT_VIEW;

  const canvasHost = document.createElement('div');
  canvasHost.className = 'entity-preview-canvas';
  host.appendChild(canvasHost);

  const overlay = renderOverlay(preview);
  host.appendChild(overlay);

  // We'd ideally pick a pixel size by container; use 480 default.
  const pxSize = 480;
  const scale = pxSize / viewSize;

  const stage = new K.Stage({ container: canvasHost, width: pxSize, height: pxSize });
  const layer = new K.Layer();
  stage.add(layer);

  const cx = pxSize / 2;
  const cy = pxSize / 2;
  const w2s = (v) => v * scale;

  // ── Region shape ──────────────────────────────────────────────────────
  if (preview.regionShape) {
    drawRegionShape(K, layer, preview.regionShape, cx, cy, w2s);
  }

  // ── Asteroid-field donut ──────────────────────────────────────────────
  if (preview.asteroidField) {
    drawDonut(
      K, layer, cx, cy,
      w2s(preview.asteroidField.innerRadius || 0),
      w2s(preview.asteroidField.outerRadius || 0),
      '#aa9966', 0.25,
    );
  }

  // ── Collider ──────────────────────────────────────────────────────────
  if (preview.colliderShape) {
    drawCollider(K, layer, preview, cx, cy, w2s);
  }

  // ── Radar shape ───────────────────────────────────────────────────────
  if (preview.radarShape) {
    drawRadarShape(K, layer, preview, cx, cy, w2s);
  }

  // ── Forward arrow ─────────────────────────────────────────────────────
  if (preview.showForwardArrow) {
    drawForwardArrow(K, layer, cx, cy);
  }

  layer.draw?.();
  return stage;
}

function computeExtent(p) {
  let max = 0;
  if (p.colliderRadius) max = Math.max(max, Math.abs(p.colliderRadius));
  if (p.colliderLength) max = Math.max(max, Math.abs(p.colliderLength) / 2);
  if (p.radarRadius) max = Math.max(max, Math.abs(p.radarRadius));
  const rs = p.regionShape;
  if (rs) {
    if (rs.type === 'sphere' && rs.radius) max = Math.max(max, Math.abs(rs.radius));
    if (rs.type === 'torus' && rs.outerRadius) max = Math.max(max, Math.abs(rs.outerRadius));
    if (rs.type === 'box' && Array.isArray(rs.halfExtents)) {
      for (const v of rs.halfExtents) max = Math.max(max, Math.abs(v));
    }
  }
  if (p.asteroidField?.outerRadius) max = Math.max(max, Math.abs(p.asteroidField.outerRadius));
  return max;
}

function drawRegionShape(K, layer, rs, cx, cy, w2s) {
  if (rs.type === 'sphere' && rs.radius != null) {
    layer.add(new K.Circle({
      x: cx, y: cy, radius: w2s(rs.radius),
      stroke: '#88aaff', strokeWidth: 1.5, dash: [4, 3], opacity: 0.7,
    }));
  } else if (rs.type === 'box' && Array.isArray(rs.halfExtents)) {
    const w = w2s(rs.halfExtents[0] || 0) * 2;
    const h = w2s(rs.halfExtents[2] ?? rs.halfExtents[1] ?? 0) * 2;
    layer.add(new K.Rect({
      x: cx - w / 2, y: cy - h / 2,
      width: w, height: h,
      rotation: ((rs.yaw || 0) * 180) / Math.PI,
      stroke: '#88aaff', strokeWidth: 1.5, dash: [4, 3], opacity: 0.7,
    }));
  } else if (rs.type === 'torus') {
    drawDonut(K, layer, cx, cy, w2s(rs.innerRadius || 0), w2s(rs.outerRadius || 0), '#88aaff', 0.35);
  }
}

function drawDonut(K, layer, cx, cy, innerR, outerR, fill, opacity) {
  layer.add(new K.Ring({
    x: cx, y: cy,
    innerRadius: innerR, outerRadius: outerR,
    fill, opacity, stroke: fill, strokeWidth: 1,
  }));
}

function drawCollider(K, layer, p, cx, cy, w2s) {
  const shape = p.colliderShape;
  const r = w2s(p.colliderRadius || 0);
  if (shape === 'Ball') {
    layer.add(new K.Circle({
      x: cx, y: cy, radius: r,
      stroke: '#ffaa66', strokeWidth: 1.5, opacity: 0.9,
    }));
  } else if (shape === 'Capsule') {
    const half = w2s((p.colliderLength || 0) / 2);
    // Pill: two semicircles + a rect, oriented along Z (which we draw as Y).
    layer.add(new K.Line({
      points: [cx - r, cy - half, cx + r, cy - half],
      stroke: '#ffaa66', strokeWidth: 1.5,
    }));
    layer.add(new K.Line({
      points: [cx - r, cy + half, cx + r, cy + half],
      stroke: '#ffaa66', strokeWidth: 1.5,
    }));
    layer.add(new K.Arc({
      x: cx, y: cy - half, innerRadius: r, outerRadius: r,
      angle: 180, rotation: 180,
      stroke: '#ffaa66', strokeWidth: 1.5,
    }));
    layer.add(new K.Arc({
      x: cx, y: cy + half, innerRadius: r, outerRadius: r,
      angle: 180, rotation: 0,
      stroke: '#ffaa66', strokeWidth: 1.5,
    }));
  }
}

function drawRadarShape(K, layer, p, cx, cy, w2s) {
  const shape = p.radarShape;
  const r = w2s(p.radarRadius || 4);
  const fill = colourToHex(p.radarColour) ?? '#cccccc';
  if (shape === RADAR_SHAPE.Triangle) {
    layer.add(new K.RegularPolygon({
      x: cx, y: cy, sides: 3, radius: r, fill, rotation: 0,
    }));
  } else if (shape === RADAR_SHAPE.Diamond) {
    layer.add(new K.RegularPolygon({
      x: cx, y: cy, sides: 4, radius: r, fill, rotation: 45,
    }));
  } else if (shape === RADAR_SHAPE.Ring) {
    layer.add(new K.Ring({
      x: cx, y: cy, innerRadius: r * 0.6, outerRadius: r, fill,
    }));
  } else if (shape === RADAR_SHAPE.Square) {
    layer.add(new K.Rect({
      x: cx - r, y: cy - r, width: r * 2, height: r * 2, fill,
    }));
  } else {
    layer.add(new K.Circle({ x: cx, y: cy, radius: Math.max(r, 2), fill }));
  }
}

function drawForwardArrow(K, layer, cx, cy) {
  // -Z is forward; we draw Y-up so the arrow points up (negative pixel Y).
  layer.add(new K.RegularPolygon({
    x: cx, y: cy - 14, sides: 3, radius: 6, fill: '#33ff66', rotation: 0,
  }));
}

function renderOverlay(preview) {
  const overlay = document.createElement('div');
  overlay.className = 'entity-preview-overlay';

  const t = preview.textOverlay || {};

  const tagsRow = document.createElement('div');
  tagsRow.className = 'entity-preview-overlay-row';
  tagsRow.textContent = `tags: ${Array.isArray(t.tags) ? t.tags.join(', ') : ''}`;
  overlay.appendChild(tagsRow);

  if (t.faction != null) {
    const fRow = document.createElement('div');
    fRow.className = 'entity-preview-overlay-row';
    fRow.textContent = `faction: ${t.faction}`;
    overlay.appendChild(fRow);
  }

  if (Array.isArray(t.consoles) && t.consoles.length > 0) {
    const cRow = document.createElement('div');
    cRow.className = 'entity-preview-overlay-row';
    cRow.textContent = `consoles: ${t.consoles.join(', ')}`;
    overlay.appendChild(cRow);
  }

  if (t.hullTotal != null) {
    const hRow = document.createElement('div');
    hRow.className = 'entity-preview-overlay-row';
    hRow.textContent = `hull: ${t.hullTotal}`;
    overlay.appendChild(hRow);
  }

  return overlay;
}

function colourToHex(c) {
  if (!Array.isArray(c) || c.length < 3) return null;
  const to255 = (v) => Math.max(0, Math.min(255, Math.round(Number(v) * 255)));
  const r = to255(c[0]).toString(16).padStart(2, '0');
  const g = to255(c[1]).toString(16).padStart(2, '0');
  const b = to255(c[2]).toString(16).padStart(2, '0');
  return `#${r}${g}${b}`;
}
