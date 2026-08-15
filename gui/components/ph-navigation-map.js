// strings-boot first: its top-level await delays this module's evaluation —
// and therefore this element's registration and upgrade — until the string
// table is loaded, so the constructor's template t() calls never see an
// empty table. No-op in Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { t } from '../strings.js';
import { phAdoptConsoleStyles } from './ph-console-styles.js';

export class PhNavigationMap extends HTMLElement {
  #state = null;
  #canvas = null;
  #ctx = null;
  #offscreen = null;
  #rafId = null;
  #resizeObserver = null;
  #needsRender = true;
  #projectedBlips = [];
  #selectedBlip = null;
  #overlay = null;
  #toast = null;
  #toastTimer = null;
  #btnSetWaypoint = null;
  #btnSetSelected = null;
  #btnClearWaypoint = null;
  #picking = false;

  #zoom = 1;
  #panX = 0;
  #panY = 0;
  #ZOOM_MIN = 0.25;
  #ZOOM_MAX = 8;

  #isDragging = false;
  #tapMoved = false;
  #dragStartX = 0;
  #dragStartY = 0;
  #startPanX = 0;
  #startPanY = 0;

  #lastPinchDist = 0;
  #pinchMidX = 0;
  #pinchMidY = 0;

  #touchActive = false;

  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    // Every component adopts the shared control family (module 1 of PRD
    // #1023): custom properties cross a shadow boundary, class rules do not.
    phAdoptConsoleStyles(this.shadowRoot);
    const tpl = document.createElement('template');
    tpl.innerHTML = [
      '<style>',
      ':host { display: block; position: relative; touch-action: none; }',
      'canvas { display: block; width: 100%; height: 100%; touch-action: none; }',
      'canvas.picking { cursor: crosshair; }',
      '#overlay {',
      '  position: absolute; bottom: 0; left: 0; right: 0;',
      '  padding: 10px 14px 14px;',
      '  background: linear-gradient(0deg, rgba(5,8,24,0.95) 0%, rgba(5,8,24,0.7) 70%, transparent 100%);',
      '  border-top: 1px solid rgba(40,50,80,0.45);',
      '  font-family: "JetBrains Mono", monospace;',
      '  pointer-events: none; display: none;',
      '}',
      '#overlay.show { display: block; }',
      '#overlay .ov-name { font-size: 16px; color: var(--ink); font-weight: 600; letter-spacing: 0.05em; }',
      '#overlay .ov-detail { font-size: 11px; color: var(--ink-dim); margin-top: 3px; letter-spacing: 0.15em; display: flex; gap: 10px; }',
      '.st-hostile { color: #ff6040; }',
      '.st-friendly { color: var(--loaded); }',
      '.st-neutral { color: #7a90c0; }',
      '.st-unknown { color: var(--ink-dim); }',
      '.wp-bar { position: absolute; top: 10px; left: 12px; right: 12px; display: flex; justify-content: flex-end; gap: 8px; z-index: 2; pointer-events: none; }',
      '.wp-btn { pointer-events: auto; font-family: "JetBrains Mono", monospace; font-size: 11px; letter-spacing: 0.14em; text-transform: uppercase; color: var(--cyan, #6cb6d0); background: rgba(10,14,28,0.88); border: 1px solid rgba(70,95,165,0.5); padding: 6px 12px; cursor: pointer; display: none; white-space: nowrap; }',
      '.wp-btn.show { display: block; }',
      '.wp-btn.active { color: #d4a820; border-color: #d4a820; }',
      '.wp-btn:active { opacity: 0.7; }',
      '.toast { position: absolute; top: 50%; left: 50%; transform: translate(-50%, -50%); font-family: "JetBrains Mono", monospace; font-size: 12px; letter-spacing: 0.3em; color: var(--tactical, #d4a820); background: rgba(5,8,24,0.92); border: 1px solid var(--tactical, #d4a820); box-shadow: 0 0 24px rgba(240,132,56,0.25); padding: 10px 24px; pointer-events: none; opacity: 0; transition: opacity 0.22s ease; z-index: 3; }',
      '.toast.show { opacity: 1; }',
      '</style>',
      '<div style="position:relative;width:100%;height:100%">',
      '  <canvas></canvas>',
      '  <div class="wp-bar">',
      '    <button type="button" class="wp-btn" id="btn-set-waypoint">' + t('console.navigation.set_waypoint') + '</button>',
      '    <button type="button" class="wp-btn" id="btn-set-selected">' + t('console.navigation.set_as_waypoint') + '</button>',
      '    <button type="button" class="wp-btn" id="btn-clear-waypoint">' + t('console.navigation.clear_waypoint') + '</button>',
      '  </div>',
      '  <div class="toast" id="toast" role="status"></div>',
      '  <div id="overlay">',
      '    <div class="ov-name" id="ov-name"></div>',
      '    <div class="ov-detail">',
      '      <span id="ov-kind"></span>',
      '      <span id="ov-stance" class="st-unknown"></span>',
      '    </div>',
      '  </div>',
      '</div>',
    ].join('\n');
    this.shadowRoot.appendChild(tpl.content.cloneNode(true));
    this.#canvas = this.shadowRoot.querySelector('canvas');
    this.#ctx = this.#canvas.getContext('2d', { alpha: false });
    this.#overlay = this.shadowRoot.getElementById('overlay');
    this.#toast = this.shadowRoot.getElementById('toast');
    this.#btnSetWaypoint = this.shadowRoot.getElementById('btn-set-waypoint');
    this.#btnSetSelected = this.shadowRoot.getElementById('btn-set-selected');
    this.#btnClearWaypoint = this.shadowRoot.getElementById('btn-clear-waypoint');
    this.#btnSetWaypoint.addEventListener('click', (e) => { e.stopPropagation(); this.#beginPick(); });
    this.#btnSetSelected.addEventListener('click', (e) => { e.stopPropagation(); this.#setToSelected(); });
    this.#btnClearWaypoint.addEventListener('click', (e) => { e.stopPropagation(); this.#clearWaypoint(); });
    this.#initResize();
    this.#rafLoop();
  }

  connectedCallback() {
    this.sendAction ??= window.sendAction;
    this.#canvas.addEventListener('mousedown', this.#boundMouseDown);
    this.#canvas.addEventListener('mousemove', this.#boundMouseMove);
    this.#canvas.addEventListener('mouseup', this.#boundMouseUp);
    this.#canvas.addEventListener('mouseleave', this.#boundMouseUp);
    this.#canvas.addEventListener('wheel', this.#boundWheel, { passive: false });
    this.#canvas.addEventListener('touchstart', this.#boundTouchStart, { passive: false });
    this.#canvas.addEventListener('touchmove', this.#boundTouchMove, { passive: false });
    this.#canvas.addEventListener('touchend', this.#boundTouchEnd);
    this.#canvas.addEventListener('touchcancel', this.#boundTouchEnd);
  }

  disconnectedCallback() {
    if (this.#rafId != null) {
      cancelAnimationFrame(this.#rafId);
      this.#rafId = null;
    }
    if (this.#resizeObserver) {
      this.#resizeObserver.disconnect();
      this.#resizeObserver = null;
    }
    this.#canvas.removeEventListener('mousedown', this.#boundMouseDown);
    this.#canvas.removeEventListener('mousemove', this.#boundMouseMove);
    this.#canvas.removeEventListener('mouseup', this.#boundMouseUp);
    this.#canvas.removeEventListener('mouseleave', this.#boundMouseUp);
    this.#canvas.removeEventListener('wheel', this.#boundWheel);
    this.#canvas.removeEventListener('touchstart', this.#boundTouchStart);
    this.#canvas.removeEventListener('touchmove', this.#boundTouchMove);
    this.#canvas.removeEventListener('touchend', this.#boundTouchEnd);
    this.#canvas.removeEventListener('touchcancel', this.#boundTouchEnd);
  }

  set state(val) {
    this.#state = val;
    this.#needsRender = true;
  }

  get state() { return this.#state; }

  #initResize() {
    const updateSize = () => {
      const dpr = window.devicePixelRatio || 1;
      const rect = this.getBoundingClientRect();
      const newW = Math.round(rect.width * dpr);
      const newH = Math.round(rect.height * dpr);
      if (newW > 0 && newH > 0 && (this.#canvas.width !== newW || this.#canvas.height !== newH)) {
        this.#canvas.width = newW;
        this.#canvas.height = newH;
        this.#needsRender = true;
      }
    };
    updateSize();
    this.#resizeObserver = new ResizeObserver(() => updateSize());
    this.#resizeObserver.observe(this);
  }

  #rafLoop() {
    if (this.#needsRender) this.#render();
    this.#rafId = requestAnimationFrame(() => this.#rafLoop());
  }

  #worldToScreen(wx, wz, shipX, shipZ, headingRad, scale, cx, cy) {
    const dx = wx - shipX;
    const dz = wz - shipZ;
    const rx = dx * Math.cos(headingRad) + dz * Math.sin(headingRad);
    const rz = dx * Math.sin(headingRad) - dz * Math.cos(headingRad);
    return [
      cx + this.#panX + rx * scale * this.#zoom,
      cy + this.#panY + rz * scale * this.#zoom,
    ];
  }

  #screenToWorld(sx, sy, shipX, shipZ, headingRad, scale, cx, cy) {
    const nx = (sx - cx - this.#panX) / (scale * this.#zoom);
    const ny = (sy - cy - this.#panY) / (scale * this.#zoom);
    const cosH = Math.cos(headingRad);
    const sinH = Math.sin(headingRad);
    return [
      nx * cosH + ny * sinH + shipX,
      nx * sinH - ny * cosH + shipZ,
    ];
  }

  #eventBufPos(e) {
    const rect = this.#canvas.getBoundingClientRect();
    let src;
    if (e.touches && e.touches.length > 0) {
      src = e.touches[0];
    } else if (e.changedTouches && e.changedTouches.length > 0) {
      src = e.changedTouches[0];
    } else {
      src = e;
    }
    const cssX = src.clientX - rect.left;
    const cssY = src.clientY - rect.top;
    return {
      x: cssX * (this.#canvas.width / rect.width),
      y: cssY * (this.#canvas.height / rect.height),
    };
  }

  #render() {
    const canvas = this.#canvas;
    if (!canvas) return;
    const W = canvas.width, H = canvas.height;
    if (W === 0 || H === 0) return;
    const cx = W / 2, cy = H / 2;
    const R = Math.min(W, H) / 2;
    const state = this.#state || {};
    const blips = state.blips || [];
    const regions = state.regions || [];
    const range = state.range || 50000;
    const rangeClamped = range > 0 ? range : 50000;
    const shipPos = state.ship_pos || { x: 0, z: 0 };
    const shipHeading = state.ship_heading || 0;
    const waypoint = state.waypoint || null;
    const headingRad = shipHeading * Math.PI / 180;
    const scale = R / rangeClamped;

    // The canvas buffer is sized rect × devicePixelRatio, so fonts and blips
    // live in buffer space while the browser maps them back to CSS pixels at
    // 1/dpr. Label fonts must therefore scale with the buffer→CSS ratio or
    // they render dpr-times smaller (unreadably tiny on phones).
    const cssW = this.getBoundingClientRect ? this.getBoundingClientRect().width : 0;
    const px = (cssW > 0) ? W / cssW : 1;
    const zoomFont = Math.min(1.3, Math.max(0.85, this.#zoom));
    const namePx = Math.round(12 * px * zoomFont);
    const wpPx = Math.round(10 * px * zoomFont);

    // Drop a selection whose blip left the chart (it may have despawned or
    // fallen outside the refresh). Emit only on a change.
    if (this.#selectedBlip && !blips.some((b) => b.uuid === this.#selectedBlip.uuid)) {
      this.#selectedBlip = null;
      this.#showOverlay(null);
      this.#dispatch('navselect', null);
    }

    let octx = this.#ctx;
    if (typeof document !== 'undefined') {
      if (!this.#offscreen || this.#offscreen.width !== W || this.#offscreen.height !== H) {
        this.#offscreen = document.createElement('canvas');
        this.#offscreen.width = W;
        this.#offscreen.height = H;
      }
      octx = this.#offscreen.getContext('2d');
    }

    octx.fillStyle = '#07080c';
    octx.fillRect(0, 0, W, H);

    // World-anchored, north-up chart: the camera is fixed on the world origin
    // (0,0) rather than the ship, so the ship icon is plotted at its true
    // sector position and visibly moves as it flies — instead of being pinned
    // to the centre of the screen. Matches the legacy navigation console.
    this.#drawGrid(octx, cx, cy, scale, 0, 0, 0, W, H, rangeClamped);

    // Areas sit under every point marker, matching the viewscreen radar's
    // draw order (regions before blips) so a hull never hides inside its fill.
    const showNames = this.#zoom >= 0.4;
    this.#drawRegions(octx, regions, cx, cy, scale, namePx, showNames);

    if (waypoint && Number.isFinite(waypoint.x) && Number.isFinite(waypoint.z)) {
      this.#drawWaypoint(octx, waypoint, cx, cy, scale, wpPx);
    }

    const [shipSx, shipSy] = this.#worldToScreen(shipPos.x, shipPos.z, 0, 0, 0, scale, cx, cy);
    this.#drawShipMarker(octx, shipSx, shipSy, R, headingRad);

    this.#projectedBlips = [];
    const blipR = Math.max(3, R * 0.015);
    for (const b of blips) {
      const [sx, sy] = this.#worldToScreen(b.world_x, b.world_z, 0, 0, 0, scale, cx, cy);
      if (sx < -50 || sx > W + 50 || sy < -50 || sy > H + 50) continue;
      const color = this.#blipColor(b.stance);
      // Mission contacts wear a plain gold ring. The ticked ring is the
      // target-lock decoration on the tactical radar — a different feature.
      if (b.objective_target) this.#drawObjectiveRing(octx, sx, sy, blipR + 6);
      this.#drawBlipShape(octx, b.kind, sx, sy, blipR, color);
      if (showNames && b.name) {
        octx.font = namePx + 'px "JetBrains Mono", monospace';
        octx.fillStyle = color;
        octx.fillText(b.name, sx + blipR + 4, sy + 4);
      }
      this.#projectedBlips.push({ uuid: b.uuid, sx, sy, hitR: Math.max(14, blipR + 6), blip: b });
    }

    if (this.#offscreen) {
      this.#ctx.drawImage(this.#offscreen, 0, 0);
    }

    this.#updateBar();
    this.#needsRender = false;
  }

  #drawGrid(octx, cx, cy, scale, shipX, shipZ, headingRad, W, H, range) {
    const [tlwx, tlwz] = this.#screenToWorld(0, 0, shipX, shipZ, headingRad, scale, cx, cy);
    const [brwx, brwz] = this.#screenToWorld(W, H, shipX, shipZ, headingRad, scale, cx, cy);
    const minWX = Math.min(tlwx, brwx);
    const maxWX = Math.max(tlwx, brwx);
    const minWZ = Math.min(tlwz, brwz);
    const maxWZ = Math.max(tlwz, brwz);

    let minor, major;
    if (range > 50000) { minor = 2000; major = 10000; }
    else if (range > 10000) { minor = 500; major = 2000; }
    else if (range > 2000) { minor = 100; major = 500; }
    else { minor = 50; major = 200; }

    const cosH = Math.cos(headingRad);
    const sinH = Math.sin(headingRad);
    const toScreen = (wx, wz) => {
      const dx = wx - shipX;
      const dz = wz - shipZ;
      const rx = dx * cosH + dz * sinH;
      const rz = dx * sinH - dz * cosH;
      return [cx + this.#panX + rx * scale * this.#zoom, cy + this.#panY + rz * scale * this.#zoom];
    };

    octx.strokeStyle = 'rgba(40,58,120,0.18)';
    octx.lineWidth = 0.5;
    for (let gx = Math.ceil(minWX / minor) * minor; gx < maxWX; gx += minor) {
      const [x1, y1] = toScreen(gx, minWZ);
      const [x2, y2] = toScreen(gx, maxWZ);
      octx.beginPath(); octx.moveTo(x1, y1); octx.lineTo(x2, y2); octx.stroke();
    }
    for (let gz = Math.ceil(minWZ / minor) * minor; gz < maxWZ; gz += minor) {
      const [x1, y1] = toScreen(minWX, gz);
      const [x2, y2] = toScreen(maxWX, gz);
      octx.beginPath(); octx.moveTo(x1, y1); octx.lineTo(x2, y2); octx.stroke();
    }

    octx.strokeStyle = 'rgba(70,95,165,0.28)';
    octx.lineWidth = 0.8;
    for (let gx = Math.ceil(minWX / major) * major; gx < maxWX; gx += major) {
      const [x1, y1] = toScreen(gx, minWZ);
      const [x2, y2] = toScreen(gx, maxWZ);
      octx.beginPath(); octx.moveTo(x1, y1); octx.lineTo(x2, y2); octx.stroke();
    }
    for (let gz = Math.ceil(minWZ / major) * major; gz < maxWZ; gz += major) {
      const [x1, y1] = toScreen(minWX, gz);
      const [x2, y2] = toScreen(maxWX, gz);
      octx.beginPath(); octx.moveTo(x1, y1); octx.lineTo(x2, y2); octx.stroke();
    }
  }

  #drawShipMarker(octx, sx, sy, R, headingRad) {
    const r = Math.max(6, R * 0.025);
    octx.save();
    octx.translate(sx, sy);
    octx.strokeStyle = 'rgba(108,182,208,0.2)';
    octx.lineWidth = 1;
    octx.beginPath(); octx.arc(0, 0, r * 2.2, 0, Math.PI * 2); octx.stroke();
    // Orient the ship glyph to its heading (north-up chart, 0 rad = north/up).
    octx.rotate(headingRad || 0);
    octx.shadowColor = '#6cb6d0';
    octx.shadowBlur = 14;
    octx.fillStyle = '#6cb6d0';
    octx.beginPath();
    octx.moveTo(0, -r);
    octx.lineTo(r * 0.65, r * 0.45);
    octx.lineTo(0, r * 0.05);
    octx.lineTo(-r * 0.65, r * 0.45);
    octx.closePath();
    octx.fill();
    octx.shadowBlur = 0;
    octx.restore();
  }

  #drawWaypoint(octx, wp, cx, cy, scale, wpPx) {
    const [px, py] = this.#worldToScreen(wp.x, wp.z, 0, 0, 0, scale, cx, cy);
    const d = 8;
    octx.save();
    octx.fillStyle = 'rgba(212,168,32,0.18)';
    octx.strokeStyle = '#d4a820';
    octx.lineWidth = 1.5;
    octx.shadowColor = '#d4a820';
    octx.shadowBlur = 10;
    octx.beginPath();
    octx.moveTo(px, py - d); octx.lineTo(px + d, py);
    octx.lineTo(px, py + d); octx.lineTo(px - d, py);
    octx.closePath();
    octx.fill(); octx.stroke();
    octx.shadowBlur = 0;
    if (this.#zoom >= 0.5) {
      octx.font = wpPx + 'px "JetBrains Mono", monospace';
      octx.fillStyle = '#d4a820';
      octx.textAlign = 'left';
      octx.fillText('WP', px + d + 4, py + 3);
    }
    octx.restore();
  }

  /**
   * Region colour as `[r, g, b]` 0-255 ints. `region.color` arrives as the
   * entity's authored `[radar_appearance].region_colour` — raw 0..1 floats,
   * not a CSS string. The neutral fallback is the placeholder used until a
   * colour is authored (same value as the viewscreen radar's).
   */
  #regionRgb(color) {
    const c = (Array.isArray(color) && color.length >= 3) ? color : [0.66, 0.69, 0.75];
    return [
      Math.round((c[0] || 0) * 255),
      Math.round((c[1] || 0) * 255),
      Math.round((c[2] || 0) * 255),
    ];
  }

  /**
   * Draw region hulls and outlines under the blips.
   *
   * Shapes mirror the viewscreen radar's renderer: a sphere is a filled
   * circle, a torus is one thick stroked ring (no fill), a box is an
   * axis-aligned filled rect — box `yaw` is deliberately ignored, matching
   * the Rust renderer. An `objective_target` region strokes in the waypoint
   * gold instead of its own colour: for a sphere or box that gold outline
   * sits around the region's own fill, but a torus never has a fill to
   * begin with, so its single stroked ring renders entirely gold. That is
   * exact parity with `gui/radar-widget.js`. The Rust viewscreen renderer
   * differs: it keeps a region's own authored colour and adds a small gold
   * ring only to icon-carrying objective entities, so gold-marking region
   * shapes is a chart-side extension (known parity gap), not shared
   * behaviour.
   *
   * Geometry is authored in world units: `#worldToScreen` offsets by
   * `scale * zoom` buffer pixels per world unit, so sizes use the same
   * factor. Every extent floors at 4px so a distant or zoomed-out region
   * stays visible instead of collapsing to nothing.
   */
  #drawRegions(octx, regions, cx, cy, scale, namePx, showNames) {
    if (!regions || regions.length === 0) return;
    const pxPerWorld = scale * this.#zoom;
    octx.save();
    for (const region of regions) {
      if (!region) continue;
      const [sx, sy] = this.#worldToScreen(region.x || 0, region.z || 0, 0, 0, 0, scale, cx, cy);
      const [r, g, b] = this.#regionRgb(region.color);
      const fill = 'rgba(' + r + ',' + g + ',' + b + ',0.3)';
      const stroke = region.objective_target ? '#d4a820' : 'rgb(' + r + ',' + g + ',' + b + ')';
      let labelOffset;

      if (region.shape === 'sphere') {
        const rPx = Math.max(4, (region.radius || 0) * pxPerWorld);
        octx.beginPath();
        octx.arc(sx, sy, rPx, 0, Math.PI * 2);
        octx.fillStyle = fill;
        octx.fill();
        octx.strokeStyle = stroke;
        octx.lineWidth = 1.5;
        octx.stroke();
        labelOffset = rPx;
      } else if (region.shape === 'torus') {
        const outerR = region.outer_radius != null ? region.outer_radius : (region.radius || 0);
        const outerPx = Math.max(4, outerR * pxPerWorld);
        let innerPx = Math.max(0, (region.inner_radius || 0) * pxPerWorld);
        if (innerPx >= outerPx) innerPx = Math.max(0, outerPx - 1);
        octx.beginPath();
        octx.arc(sx, sy, (outerPx + innerPx) / 2, 0, Math.PI * 2);
        octx.lineWidth = Math.max(1, outerPx - innerPx);
        octx.strokeStyle = stroke;
        octx.stroke();
        labelOffset = outerPx;
      } else if (region.shape === 'box') {
        const he = region.half_extents || [0, 0];
        const halfW = Math.max(4, (he[0] || 0) * pxPerWorld);
        const halfH = Math.max(4, (he[1] || 0) * pxPerWorld);
        octx.fillStyle = fill;
        octx.fillRect(sx - halfW, sy - halfH, halfW * 2, halfH * 2);
        octx.strokeStyle = stroke;
        octx.lineWidth = 1.5;
        octx.strokeRect(sx - halfW, sy - halfH, halfW * 2, halfH * 2);
        labelOffset = halfW;
      } else {
        // Unknown shape — skip, matching the viewscreen radar's own `_ => None`.
        continue;
      }

      // Same label treatment as blip names, including the zoom-out floor.
      if (showNames && region.name) {
        octx.font = namePx + 'px "JetBrains Mono", monospace';
        octx.fillStyle = stroke;
        octx.fillText(region.name, sx + labelOffset + 4, sy + 4);
      }
    }
    octx.restore();
  }

  /** Plain gold ring marking an objective contact (no target-lock ticks). */
  #drawObjectiveRing(octx, sx, sy, ringR) {
    octx.save();
    octx.strokeStyle = '#d4a820';
    octx.lineWidth = 2;
    octx.beginPath();
    octx.arc(sx, sy, ringR, 0, Math.PI * 2);
    octx.stroke();
    octx.restore();
  }

  #blipColor(stance) {
    if (stance === 'hostile') return '#ff6040';
    if (stance === 'friendly') return '#4ec870';
    if (stance === 'neutral') return '#7a90c0';
    return '#a8b0c0';
  }

  #drawBlipShape(octx, kind, sx, sy, r, color) {
    octx.save();
    octx.fillStyle = color;
    if (kind === 'star') {
      const spikes = 5;
      const outerR = r * 1.4;
      const innerR = r * 0.6;
      octx.beginPath();
      for (let i = 0; i < spikes * 2; i++) {
        const angle = (i * Math.PI) / spikes - Math.PI / 2;
        const rad = i % 2 === 0 ? outerR : innerR;
        const px = sx + rad * Math.cos(angle);
        const py = sy + rad * Math.sin(angle);
        if (i === 0) octx.moveTo(px, py);
        else octx.lineTo(px, py);
      }
      octx.closePath(); octx.fill();
    } else if (kind === 'station') {
      const half = r * 1.1;
      octx.fillRect(sx - half, sy - half, half * 2, half * 2);
    } else if (kind === 'ship') {
      const s = r * 1.3;
      octx.beginPath();
      octx.moveTo(sx, sy - s);
      octx.lineTo(sx + s * 0.6, sy + s * 0.5);
      octx.lineTo(sx, sy + s * 0.05);
      octx.lineTo(sx - s * 0.6, sy + s * 0.5);
      octx.closePath(); octx.fill();
    } else if (kind === 'asteroid') {
      octx.beginPath();
      octx.arc(sx, sy, r * 0.5, 0, Math.PI * 2);
      octx.fill();
    } else {
      octx.beginPath();
      octx.arc(sx, sy, r, 0, Math.PI * 2);
      octx.fill();
    }
    octx.restore();
  }

  #showOverlay(blip) {
    const nameEl = this.shadowRoot.getElementById('ov-name');
    const kindEl = this.shadowRoot.getElementById('ov-kind');
    const stanceEl = this.shadowRoot.getElementById('ov-stance');
    if (!blip) {
      this.#overlay.classList.remove('show');
      return;
    }
    nameEl.textContent = blip.name || blip.uuid || t('console.common.unknown');
    kindEl.textContent = (blip.kind || 'unknown').toUpperCase();
    stanceEl.textContent = blip.stance ? t('console.stance.' + blip.stance) : t('console.common.unknown');
    stanceEl.className = 'st-' + (blip.stance || 'unknown');
    this.#overlay.classList.add('show');
  }

  #getBlipAt(canvasX, canvasY) {
    let best = null;
    let bestDist = Infinity;
    for (const b of this.#projectedBlips) {
      const dist = Math.hypot(canvasX - b.sx, canvasY - b.sy);
      if (dist <= b.hitR && dist < bestDist) {
        best = b;
        bestDist = dist;
      }
    }
    return best;
  }

  #handleTap(bufX, bufY) {
    const state = this.#state || {};
    const range = state.range || 50000;
    const rangeClamped = range > 0 ? range : 50000;
    const R = Math.min(this.#canvas.width, this.#canvas.height) / 2;
    const scale = R / rangeClamped;
    const cx = this.#canvas.width / 2;
    const cy = this.#canvas.height / 2;

    // World-anchored chart: map the tapped screen point to its absolute world
    // coordinate (camera fixed on the origin, north-up), matching #render.
    const [wx, wz] = this.#screenToWorld(bufX, bufY, 0, 0, 0, scale, cx, cy);

    // Explicit pick mode: the next tap places a free waypoint wherever it
    // lands, regardless of whether a blip is underneath (legacy behaviour).
    if (this.#picking) {
      this.#picking = false;
      this.#canvas.classList.remove('picking');
      this.#selectedBlip = null;
      this.#showOverlay(null);
      this.#dispatch('navselect', null);
      this.#updateBar();
      if (this.sendAction) {
        this.sendAction('set_navigation_waypoint', { x: wx, z: wz });
        this.#showToast(t('console.navigation.waypoint_set'));
      }
      return;
    }

    // Tapping a chart blip selects it (opens the info overlay); it does NOT
    // set the waypoint — that is an explicit command (bar buttons), matching
    // the former navigation console. Tapping empty space clears selection.
    const hit = this.#getBlipAt(bufX, bufY);
    if (hit) {
      if (this.#selectedBlip && this.#selectedBlip.uuid === hit.blip.uuid) return;
      this.#selectedBlip = hit.blip;
      this.#showOverlay(hit.blip);
    } else {
      if (!this.#selectedBlip) return;
      this.#selectedBlip = null;
      this.#showOverlay(null);
    }
    this.#updateBar();
    this.#dispatch('navselect', this.#selectedBlip);
  }

  #beginPick() {
    this.#picking = !this.#picking;
    if (this.#picking) {
      this.#selectedBlip = null;
      this.#showOverlay(null);
      this.#dispatch('navselect', null);
      this.#canvas.classList.add('picking');
      this.#showToast(t('console.navigation.tap_to_place'), 4000);
    } else {
      this.#canvas.classList.remove('picking');
    }
    this.#updateBar();
  }

  #setToSelected() {
    const b = this.#selectedBlip;
    if (!b || !this.sendAction) return;
    // Anchor the waypoint to the selected entity's UUID; the server refreshes
    // x/z from the entity's live transform each tick and auto-clears the
    // waypoint if the entity despawns (matching the former console).
    this.sendAction('set_navigation_waypoint', {
      x: b.world_x,
      z: b.world_z,
      source_uuid: b.uuid,
    });
    this.#showToast(t('console.navigation.waypoint_set'));
  }

  #clearWaypoint() {
    if (this.sendAction) this.sendAction('clear_navigation_waypoint', {});
  }

  #updateBar() {
    const state = this.#state || {};
    const wp = state.waypoint;
    const hasWp = !!(wp && Number.isFinite(wp.x) && Number.isFinite(wp.z));
    const hasSel = !!this.#selectedBlip;
    this.#btnSetWaypoint.classList.toggle('show', !hasWp);
    this.#btnSetSelected.classList.toggle('show', hasSel);
    this.#btnClearWaypoint.classList.toggle('show', hasWp);
    this.#btnSetWaypoint.classList.toggle('active', this.#picking);
  }

  #showToast(msg, duration) {
    if (!this.#toast) return;
    this.#toast.textContent = msg;
    this.#toast.classList.add('show');
    clearTimeout(this.#toastTimer);
    this.#toastTimer = setTimeout(() => this.#toast.classList.remove('show'), duration || 1900);
  }

  #dispatch(type, detail) {
    this.dispatchEvent(new CustomEvent(type, { bubbles: true, composed: true, detail: detail }));
  }

  #onPointerTap(e) {
    if (!this.#canvas) return;
    const cpos = this.#eventBufPos(e);
    this.#handleTap(cpos.x, cpos.y);
  }

  #boundMouseDown = (e) => {
    if (this.#touchActive) return;
    const cpos = this.#eventBufPos(e);
    this.#isDragging = true;
    this.#tapMoved = false;
    this.#dragStartX = cpos.x;
    this.#dragStartY = cpos.y;
    this.#startPanX = this.#panX;
    this.#startPanY = this.#panY;
  };

  #boundMouseMove = (e) => {
    if (!this.#isDragging) return;
    const cpos = this.#eventBufPos(e);
    const dx = cpos.x - this.#dragStartX;
    const dy = cpos.y - this.#dragStartY;
    if (Math.hypot(dx, dy) > 5) {
      this.#tapMoved = true;
      this.#panX = this.#startPanX + dx;
      this.#panY = this.#startPanY + dy;
      this.#needsRender = true;
    }
  };

  #boundMouseUp = (e) => {
    if (!this.#isDragging) return;
    this.#isDragging = false;
    if (!this.#tapMoved) {
      this.#handleTap(this.#dragStartX, this.#dragStartY);
    }
  };

  #boundWheel = (e) => {
    e.preventDefault();
    const cpos = this.#eventBufPos(e);
    const factor = e.deltaY < 0 ? 1.13 : 0.885;
    const newZoom = Math.max(this.#ZOOM_MIN, Math.min(this.#ZOOM_MAX, this.#zoom * factor));
    const canvas = this.#canvas;
    const cx = canvas.width / 2;
    const cy = canvas.height / 2;
    const zoomRatio = newZoom / this.#zoom;
    this.#panX = cpos.x - cx - (cpos.x - cx - this.#panX) * zoomRatio;
    this.#panY = cpos.y - cy - (cpos.y - cy - this.#panY) * zoomRatio;
    this.#zoom = newZoom;
    this.#needsRender = true;
  };

  #boundTouchStart = (e) => {
    this.#touchActive = true;
    if (e.touches.length === 1) {
      const cpos = this.#eventBufPos(e);
      this.#isDragging = true;
      this.#tapMoved = false;
      this.#dragStartX = cpos.x;
      this.#dragStartY = cpos.y;
      this.#startPanX = this.#panX;
      this.#startPanY = this.#panY;
    } else if (e.touches.length === 2) {
      this.#isDragging = false;
      this.#tapMoved = true;
      const t0 = e.touches[0], t1 = e.touches[1];
      this.#lastPinchDist = Math.hypot(t0.clientX - t1.clientX, t0.clientY - t1.clientY);
      const rect = this.#canvas.getBoundingClientRect();
      const mx = ((t0.clientX + t1.clientX) / 2 - rect.left) * (this.#canvas.width / rect.width);
      const my = ((t0.clientY + t1.clientY) / 2 - rect.top) * (this.#canvas.height / rect.height);
      this.#pinchMidX = mx;
      this.#pinchMidY = my;
    }
  };

  #boundTouchMove = (e) => {
    e.preventDefault();
    if (e.touches.length === 2 && this.#lastPinchDist > 0) {
      const t0 = e.touches[0], t1 = e.touches[1];
      const dist = Math.hypot(t0.clientX - t1.clientX, t0.clientY - t1.clientY);
      const factor = dist / this.#lastPinchDist;
      const newZoom = Math.max(this.#ZOOM_MIN, Math.min(this.#ZOOM_MAX, this.#zoom * factor));
      const canvas = this.#canvas;
      const cx = canvas.width / 2;
      const cy = canvas.height / 2;
      const zoomRatio = newZoom / this.#zoom;
      this.#panX = this.#pinchMidX - cx - (this.#pinchMidX - cx - this.#panX) * zoomRatio;
      this.#panY = this.#pinchMidY - cy - (this.#pinchMidY - cy - this.#panY) * zoomRatio;
      this.#zoom = newZoom;
      this.#lastPinchDist = dist;
      this.#needsRender = true;
    } else if (e.touches.length === 1 && this.#isDragging) {
      const cpos = this.#eventBufPos(e);
      const dx = cpos.x - this.#dragStartX;
      const dy = cpos.y - this.#dragStartY;
      if (Math.hypot(dx, dy) > 5) {
        this.#tapMoved = true;
        this.#panX = this.#startPanX + dx;
        this.#panY = this.#startPanY + dy;
        this.#needsRender = true;
      }
    }
  };

  #boundTouchEnd = (e) => {
    if (this.#lastPinchDist > 0) {
      this.#lastPinchDist = 0;
    }
    if (this.#isDragging) {
      this.#isDragging = false;
      if (!this.#tapMoved && e.changedTouches.length > 0) {
        this.#onPointerTap(e);
      }
    }
    if (e.touches.length === 0) {
      this.#touchActive = false;
    }
  };
}

if (typeof window !== 'undefined' && !customElements.get('ph-navigation-map')) {
  customElements.define('ph-navigation-map', PhNavigationMap);
}
