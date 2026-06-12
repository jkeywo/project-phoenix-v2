/**
 * gui/radar-widget.js — Shared HTML Radar Widget (Project Phoenix)
 *
 * A reusable Canvas2D radar component for phone console HTML pages.
 *
 * Slice 1  (#444) — constructor, pre-projected mode, colored-circle blips,
 *                   range rings, background disc, own-ship triangle, hit-test.
 * Slice 2  (#446) — PNG icon blips, target ring, objective rings, fire arcs.
 * Slice 5a (#447) — world-space projection mode (radar-math.js integration).
 * Slice 5b (#449) — zoom/pan, auto-scale, text labels.
 *
 * Usage:
 *   var widget = new RadarWidget(canvas, { consoleId: 'tactical', onBlipTap: fn });
 *   widget.update({ mode: 'pre-projected', blips: [...], ... });
 *   widget.destroy();
 */
(function (root, factory) {
  'use strict';
  if (typeof module !== 'undefined' && module.exports) {
    module.exports = { RadarWidget: factory() };
  } else {
    root.RadarWidget = factory();
  }
})(typeof self !== 'undefined' ? self : this, function () {
  'use strict';

  // ── Constants ────────────────────────────────────────────────────────────

  /** Blip fill colours by entity kind (pre-projected mode, no icon). */
  var KIND_COLOR = {
    asteroid: '#7ac0ff',
    ship:     '#ff8060',
    station:  '#ffe060',
    torpedo:  '#ff60ff',
    unknown:  '#a8b0c0',
    battleship: '#e6330d',   // dark red — large enemy
    cruiser:    '#cc4d1a',   // orange-red — medium enemy
    destroyer:  '#ff3333',   // bright red — small enemy
  };

  /** Icon name → PNG filename stem (e.g. "ship" → "Icon-Ship.png"). */
  var ICON_STEMS = {
    ship:       'Ship',
    player:     'PlayerShip',
    asteroid:   'Asteroid',
    station:    'Station',
    planet:     'Planet',
    star:       'Star',
    torpedo:    'Torpedo',
    battleship: 'Battleship',
    cruiser:    'Cruiser',
    destroyer:  'Destroyer',
  };

  var MIN_BLIP_PX        = 8;    // minimum blip diameter in canvas pixels
  var MIN_RANGE          = 10;   // minimum effective range for auto-scale
  var DEFAULT_RANGE      = 300;  // default effective range in world units
  var ZOOM_MIN           = 0.25;
  var ZOOM_MAX           = 8.0;

  // ── Constructor ──────────────────────────────────────────────────────────

  /**
   * @param {HTMLCanvasElement} canvasElement
   * @param {object}  opts
   * @param {string}  [opts.consoleId='tactical']
   * @param {string}  [opts.orientation='ship_relative']
   *        'ship_relative' | 'world_fixed' | 'world_centred'
   * @param {string}  [opts.clipMode='circle']  'circle' | 'square' | 'none'
   * @param {number}  [opts.range=300]   effective range in world units
   * @param {string}  [opts.iconBasePath='../assets/radar_icons/Icon-']
   * @param {boolean} [opts.autoScale=false]
   * @param {Function} [opts.onBlipTap]    (uuid: string) => void
   * @param {Function} [opts.onZoomChange] (factor: number) => void
   * @param {Function} [opts.onPanChange]  (x: number, z: number) => void
   */
  function RadarWidget(canvasElement, opts) {
    if (!canvasElement || canvasElement.nodeName !== 'CANVAS') {
      throw new Error('RadarWidget: first argument must be a <canvas> element');
    }
    opts = opts || {};

    this._canvas       = canvasElement;
    this._ctx          = canvasElement.getContext('2d');
    this._consoleId    = opts.consoleId    || 'tactical';
    this._orientation  = opts.orientation  || 'ship_relative';
    this._clipMode     = opts.clipMode     || 'circle';
    this._range        = opts.range        || DEFAULT_RANGE;
    this._iconBasePath = opts.iconBasePath || '../assets/radar_icons/Icon-';
    this._autoScale    = !!opts.autoScale;

    this._onBlipTap    = typeof opts.onBlipTap    === 'function' ? opts.onBlipTap    : null;
    this._onZoomChange = typeof opts.onZoomChange === 'function' ? opts.onZoomChange : null;
    this._onPanChange  = typeof opts.onPanChange  === 'function' ? opts.onPanChange  : null;

    // radar-math.js functions — injected via opts.math or window.RadarMath.
    // Required for world-space projection mode (Slice 5a / #447).
    this._math = opts.math ||
      (typeof window !== 'undefined' && window.RadarMath) ||
      null;

    this._data            = null;
    this._rafId           = null;
    this._destroyed       = false;
    this._projectedBlips  = null;  // cache for world-space hit-testing

    // View state (zoom / pan) — Slice 5b (#449)
    this._zoom = 1.0;
    this._panX = 0.0;
    this._panZ = 0.0;

    // Gesture tracking state (Slice 5b / #449)
    this._pointers      = {};    // active pointers: id → {x, y}
    this._lastPinchDist = null;  // last two-finger distance (pinch-zoom)
    this._dragStart     = null;  // {x, y, panX, panZ, worldPerCss} at drag-start
    this._didDrag       = false; // suppress click-based tap-to-lock after drag

    // Icon images pre-loaded in Slice 2 (#446)
    this._icons = {};
    this._loadIcons();

    // Enable pointer events so tap-to-lock works.
    // The console CSS may set pointer-events:none on .radar canvas; we override.
    this._canvas.style.pointerEvents = 'auto';

    // DPR-aware canvas sizing with ResizeObserver
    this._resizeObserver = null;
    this._updateSizeFn   = null;
    this._initResize();

    // rAF loop
    var self = this;
    this._rafLoop = function () { self._loop(); };
    this._rafId = requestAnimationFrame(this._rafLoop);

    // Pointer / touch event listeners — tap-to-lock
    this._boundClick      = this._onPointerTap.bind(this, 'click');
    this._boundTouchStart = this._onPointerTap.bind(this, 'touch');
    this._canvas.addEventListener('click',      this._boundClick);
    this._canvas.addEventListener('touchstart', this._boundTouchStart, { passive: false });

    // Pointer events for drag-pan and pinch-zoom (Slice 5b / #449)
    this._boundPointerDown = this._onPointerDown.bind(this);
    this._boundPointerMove = this._onPointerMove.bind(this);
    this._boundPointerUp   = this._onPointerUp.bind(this);
    this._boundDblClick    = this._onDblClick.bind(this);
    this._canvas.addEventListener('pointerdown',   this._boundPointerDown);
    this._canvas.addEventListener('pointermove',   this._boundPointerMove);
    this._canvas.addEventListener('pointerup',     this._boundPointerUp);
    this._canvas.addEventListener('pointercancel', this._boundPointerUp);
    this._canvas.addEventListener('dblclick',      this._boundDblClick);
  }

  // ── Resize / DPR ────────────────────────────────────────────────────────

  RadarWidget.prototype._initResize = function () {
    var self = this;

    function updateSize() {
      if (self._destroyed || !self._canvas) return;
      var dpr  = (typeof window !== 'undefined' && window.devicePixelRatio) || 1;
      var rect = self._canvas.getBoundingClientRect();
      var newW = Math.round(rect.width  * dpr);
      var newH = Math.round(rect.height * dpr);
      if (newW > 0 && newH > 0 &&
          (self._canvas.width !== newW || self._canvas.height !== newH)) {
        self._canvas.width  = newW;
        self._canvas.height = newH;
      }
    }

    this._updateSizeFn = updateSize;
    updateSize();

    if (typeof ResizeObserver !== 'undefined') {
      this._resizeObserver = new ResizeObserver(function () { updateSize(); });
      this._resizeObserver.observe(this._canvas);
    }
    if (typeof window !== 'undefined') {
      window.addEventListener('resize', updateSize);
    }
  };

  // ── rAF loop ─────────────────────────────────────────────────────────────

  RadarWidget.prototype._loop = function () {
    if (this._destroyed) return;
    this._render();
    this._rafId = requestAnimationFrame(this._rafLoop);
  };

  // ── Public API ────────────────────────────────────────────────────────────

  /**
   * Push new state data.  Accepts two modes:
   *   { mode: 'pre-projected', range, blips, phaser_arcs, torpedo_arcs,
   *     target_uuid, objective_uuids }
   *   { mode: 'world-space',  ship_x, ship_z, ship_yaw, orientation,
   *     effective_range, auto_scale, user_zoom, user_pan_x, user_pan_z,
   *     entities, regions, arcs, target_uuid, objective_uuids }
   */
  RadarWidget.prototype.update = function (data) {
    this._data = data;
  };

  /** Override effective range (e.g. from RadarDampening modifier). */
  RadarWidget.prototype.setRange = function (range) {
    this._range = range;
  };

  RadarWidget.prototype.setZoom = function (factor) {
    this._zoom = Math.max(ZOOM_MIN, Math.min(ZOOM_MAX, factor));
  };

  RadarWidget.prototype.setPan = function (x, z) {
    this._panX = x;
    this._panZ = z;
  };

  RadarWidget.prototype.getZoom = function () { return this._zoom; };
  RadarWidget.prototype.getPan  = function () { return { x: this._panX, z: this._panZ }; };

  /** Stop the rAF loop and remove all event listeners. */
  RadarWidget.prototype.destroy = function () {
    this._destroyed = true;

    if (this._rafId != null) {
      cancelAnimationFrame(this._rafId);
      this._rafId = null;
    }
    if (this._resizeObserver) {
      this._resizeObserver.disconnect();
      this._resizeObserver = null;
    }
    if (this._updateSizeFn && typeof window !== 'undefined') {
      window.removeEventListener('resize', this._updateSizeFn);
    }
    if (this._canvas) {
      this._canvas.removeEventListener('click',      this._boundClick);
      this._canvas.removeEventListener('touchstart', this._boundTouchStart);
      // Gesture listeners added by Slice 5b
      if (this._boundPointerDown) {
        this._canvas.removeEventListener('pointerdown', this._boundPointerDown);
        this._canvas.removeEventListener('pointermove', this._boundPointerMove);
        this._canvas.removeEventListener('pointerup',   this._boundPointerUp);
        this._canvas.removeEventListener('pointercancel', this._boundPointerUp);
        this._canvas.removeEventListener('dblclick',    this._boundDblClick);
      }
    }

    this._data            = null;
    this._canvas          = null;
    this._ctx             = null;
    this._icons           = {};
    this._projectedBlips  = null;
  };

  // ── Rendering ─────────────────────────────────────────────────────────────

  RadarWidget.prototype._render = function () {
    if (!this._canvas || !this._ctx) return;
    var ctx    = this._ctx;
    var canvas = this._canvas;
    var W = canvas.width, H = canvas.height;
    if (W === 0 || H === 0) return;
    var cx = W / 2, cy = H / 2;
    var R  = Math.min(W, H) / 2 - 8;
    var data = this._data;

    // Opaque fill prevents canvas clearRect flicker (issue #2)
    ctx.fillStyle = '#07080c';
    ctx.fillRect(0, 0, W, H);

    // ── Background disc ────────────────────────────────────────────────────
    ctx.fillStyle = 'rgba(5,8,22,0.52)';
    ctx.beginPath();
    ctx.arc(cx, cy, R, 0, Math.PI * 2);
    ctx.fill();

    // ── Range rings (33 / 66 / 100 %) ─────────────────────────────────────
    ctx.strokeStyle = 'rgba(106,124,164,0.28)';
    ctx.lineWidth   = 1;
    [0.33, 0.66, 1.0].forEach(function (f) {
      ctx.beginPath();
      ctx.arc(cx, cy, R * f, 0, Math.PI * 2);
      ctx.stroke();
    });

    // ── Fire-arc sectors (before clip so they span full radius) ───────────
    if (data) {
      if (data.mode === 'pre-projected') {
        this._drawArcSectors(ctx, cx, cy, R,
          data.torpedo_arcs, 0.70,
          'rgba(60,160,240,0.08)', 'rgba(60,160,240,0.35)');
        this._drawArcSectors(ctx, cx, cy, R,
          data.phaser_arcs, 0.90,
          'rgba(240,132,56,0.10)', 'rgba(240,132,56,0.40)');
      } else if (data.mode === 'world-space' && data.arcs) {
        this._drawWorldSpaceArcs(ctx, cx, cy, R, data.arcs);
      }
    }

    // ── Circle clip ────────────────────────────────────────────────────────
    ctx.save();
    if (this._clipMode === 'circle') {
      ctx.beginPath();
      ctx.arc(cx, cy, R, 0, Math.PI * 2);
      ctx.clip();
    }

    // ── Blips ──────────────────────────────────────────────────────────────
    if (data) {
      if (data.mode === 'pre-projected') {
        this._drawPreProjectedBlips(ctx, cx, cy, R, data);
      } else if (data.mode === 'world-space') {
        this._drawWorldSpaceBlips(ctx, cx, cy, R, data);
      }
    }

    ctx.restore();

    // ── Own-ship marker (always on top, outside clip) ─────────────────────
    this._drawOwnShip(ctx, cx, cy);
  };

  // ── Arc rendering ─────────────────────────────────────────────────────────

  RadarWidget.prototype._drawArcSectors = function (ctx, cx, cy, R, arcs, innerFrac, fillColor, strokeColor) {
    var arcR = R * innerFrac;
    (arcs || []).forEach(function (arc) {
      // facing_deg=0 → forward (up). canvas_angle = (facing_deg − 90) × π/180
      var facing  = (arc.facing_deg - 90) * Math.PI / 180;
      var arcDeg  = arc.fire_arc_deg != null ? arc.fire_arc_deg : (arc.arc_deg || 0);
      var halfArc = arcDeg * Math.PI / 180 / 2;
      ctx.beginPath();
      ctx.moveTo(cx, cy);
      ctx.arc(cx, cy, arcR, facing - halfArc, facing + halfArc);
      ctx.closePath();
      ctx.fillStyle   = fillColor;
      ctx.fill();
      ctx.strokeStyle = strokeColor;
      ctx.lineWidth   = 1;
      ctx.stroke();
    });
  };

  RadarWidget.prototype._drawWorldSpaceArcs = function (ctx, cx, cy, R, arcs) {
    (arcs || []).forEach(function (arc) {
      var isPhaser = arc.type === 'phaser';
      var innerFrac = isPhaser ? 0.90 : 0.70;
      var fill      = isPhaser ? 'rgba(240,132,56,0.10)' : 'rgba(60,160,240,0.08)';
      var stroke    = isPhaser ? 'rgba(240,132,56,0.40)'  : 'rgba(60,160,240,0.35)';
      var arcR    = R * (arc.range_frac != null ? arc.range_frac : innerFrac);
      var facing  = (arc.facing_deg - 90) * Math.PI / 180;
      var halfArc = (arc.arc_deg || 0) * Math.PI / 180 / 2;
      ctx.beginPath();
      ctx.moveTo(cx, cy);
      ctx.arc(cx, cy, arcR, facing - halfArc, facing + halfArc);
      ctx.closePath();
      ctx.fillStyle   = fill;
      ctx.fill();
      ctx.strokeStyle = stroke;
      ctx.lineWidth   = 1;
      ctx.stroke();
    });
  };

  // ── Pre-projected blip rendering ──────────────────────────────────────────

  RadarWidget.prototype._drawPreProjectedBlips = function (ctx, cx, cy, R, data) {
    var self = this;
    var targetUuid     = data.target_uuid      || null;
    var objectiveUuids = data.objective_uuids  || [];

    (data.blips || []).forEach(function (b) {
      // radar_x = starboard (+right), radar_y = forward (+up) → negate Y for canvas
      var bx = cx + b.radar_x * R;
      var by = cy - b.radar_y * R;
      var isTarget    = targetUuid && targetUuid === b.uuid;
      var isObjective = b.objective_target || objectiveUuids.indexOf(b.uuid) !== -1;
      var dotR = Math.max(MIN_BLIP_PX / 2, (b.scaled_radius || 0) * R * 0.6);

      // Objective gold ring (drawn behind blip)
      if (isObjective) {
        self._drawRing(ctx, bx, by, dotR + 6, 2, '#d4a820', false);
      }

      // Blip: PNG icon or colored circle fallback
      var iconName = b.icon || b.kind;
      var icon = self._icons[iconName];
      var iconLoaded = icon && icon.complete && icon.naturalWidth > 0;

      if (iconLoaded) {
        self._drawIconBlip(ctx, icon, bx, by, dotR, b.color);
      } else {
        // Colored circle fallback
        var color = KIND_COLOR[b.kind] || KIND_COLOR.unknown;
        ctx.beginPath();
        ctx.arc(bx, by, dotR, 0, Math.PI * 2);
        ctx.fillStyle = color;
        ctx.fill();
      }

      // Target highlight ring (red, drawn on top)
      if (isTarget) {
        self._drawRing(ctx, bx, by, dotR + 7, 2, '#ff3344', true);
      }
    });
  };

  // ── World-space blip rendering (Slice 5a / #447) ─────────────────────────

  RadarWidget.prototype._drawWorldSpaceBlips = function (ctx, cx, cy, R, data) {
    this._projectAndDrawWorldEntities(ctx, cx, cy, R, data);
  };

  /**
   * Project world-space entities via radar-math and draw them.
   * Caches projected canvas coords in this._projectedBlips for hit-testing.
   */
  RadarWidget.prototype._projectAndDrawWorldEntities = function (ctx, cx, cy, R, data) {
    var math = this._math;
    if (!math) return;

    var shipX       = data.ship_x        || 0;
    var shipZ       = data.ship_z        || 0;
    var shipYaw     = data.ship_yaw      || 0;
    var orientation = data.orientation   || this._orientation;
    var range       = data.effective_range != null ? data.effective_range : this._range;

    // Auto-scale: find minimum range that contains all entities
    if (this._autoScale || data.auto_scale) {
      var positions = (data.entities || []).map(function (e) {
        return math.worldToRadar(e.x || 0, e.z || 0, shipX, shipZ, shipYaw, orientation);
      });
      range = math.autoScaleRange(positions);
    }

    // Divide by zoom to widen the visible range when zoomed out
    var effectiveRange = range / Math.max(this._zoom, 0.001);

    // Draw regions BEFORE entities — matches Bevy ZIndex ordering (regions=3, blips=10)
    this._drawWorldSpaceRegions(ctx, cx, cy, R, data.regions || [],
      shipX, shipZ, shipYaw, orientation, effectiveRange);

    var targetUuid     = data.target_uuid     || null;
    var objectiveUuids = data.objective_uuids || [];
    var self           = this;

    // Project each entity and collect canvas coords
    var projected = [];
    (data.entities || []).forEach(function (e) {
      var rp = math.worldToRadar(
        e.x || 0, e.z || 0,
        shipX, shipZ, shipYaw, orientation
      );
      // Apply pan offset (in radar-space world units)
      var rx = rp.rx - (self._panX || 0);
      var rz = rp.rz - (self._panZ || 0);

      var sp  = math.radarToScreen(rx, rz, effectiveRange, R);
      var bx  = cx + sp.sx;
      var by  = cy + sp.sy;  // sp.sy already negated by radarToScreen
      var rawRadius = e.radius || 0;
      var dotR = Math.max(MIN_BLIP_PX / 2, (rawRadius / effectiveRange) * R * 0.6);

      projected.push({
        bx: bx, by: by, dotR: dotR,
        uuid:             e.uuid,
        icon:             e.icon,
        kind:             e.kind,
        color:            e.color,
        objective_target: e.objective_target,
        name:             e.name,
        // sx/sy stored for _getBlipAt
        sx: sp.sx, sy: sp.sy,
        scaled_radius: effectiveRange > 0 ? rawRadius / effectiveRange : 0,
      });
    });

    // Cache for hit-testing (_getBlipAt uses _bx/_by when present)
    this._projectedBlips = projected;

    // Draw
    projected.forEach(function (p) {
      var bx = p.bx, by = p.by, dotR = p.dotR;
      var isTarget    = targetUuid && targetUuid === p.uuid;
      var isObjective = p.objective_target || objectiveUuids.indexOf(p.uuid) !== -1;

      if (isObjective) {
        self._drawRing(ctx, bx, by, dotR + 6, 2, '#d4a820', false);
      }

      var iconName = p.icon || p.kind;
      var icon     = self._icons[iconName];
      var iconLoaded = icon && icon.complete && icon.naturalWidth > 0;

      if (iconLoaded) {
        self._drawIconBlip(ctx, icon, bx, by, dotR, p.color);
      } else {
        ctx.beginPath();
        ctx.arc(bx, by, dotR, 0, Math.PI * 2);
        ctx.fillStyle = KIND_COLOR[p.kind] || KIND_COLOR.unknown;
        ctx.fill();
      }

      if (isTarget) {
        self._drawRing(ctx, bx, by, dotR + 7, 2, '#ff3344', true);
      }

      // Text label — rendered when entity has a name (world-space mode)
      if (p.name) {
        ctx.font      = '11px "JetBrains Mono", monospace';
        ctx.fillStyle = 'rgba(153,255,217,0.9)';  // spec: rgba(0.6, 1.0, 0.85, 0.9)
        ctx.fillText(p.name, bx + dotR + 4, by + 4);
      }
    });
  };

  // ── Region shape rendering (Slice 4 / PRD #443) ─────────────────────────────
  //
  // Parity with Bevy `GenericRadarWidget` region rendering (src/gui/radar.rs).
  // Draw order:  regions FIRST (before entity blips), inside the circle clip.
  // Shape variants: 'sphere' | 'box' | 'torus'  (any other shape is skipped).
  //
  // Colour convention (matching Bevy):
  //   Sphere fill:  region.color @ 0.30 alpha
  //   Torus fill:   none (Color::NONE in Bevy)
  //   Box fill:     region.color @ 0.30 alpha
  //   All stroke:   region.color @ 1.00 alpha, 1.5 px

  /**
   * Project and draw all regions from the world-space data payload.
   *
   * @param {CanvasRenderingContext2D} ctx
   * @param {number} cx      Canvas centre X (pixels)
   * @param {number} cy      Canvas centre Y (pixels)
   * @param {number} R       Inscribed radar circle radius (canvas buffer pixels)
   * @param {Array}  regions data.regions array (RadarRegion wire objects)
   * @param {number} shipX   World X of radar origin (ship or world-centre)
   * @param {number} shipZ   World Z of radar origin
   * @param {number} shipYaw Ship heading in radians (ignored for world_fixed/world_centred)
   * @param {string} orientation 'ship_relative' | 'world_fixed' | 'world_centred'
   * @param {number} effectiveRange Effective range in world units (range / zoom)
   */
  RadarWidget.prototype._drawWorldSpaceRegions = function (
    ctx, cx, cy, R, regions, shipX, shipZ, shipYaw, orientation, effectiveRange
  ) {
    var math = this._math;
    if (!math || !regions || regions.length === 0) return;

    var scale = R / Math.max(effectiveRange, 0.001);  // canvas pixels per world unit
    var self  = this;

    regions.forEach(function (region) {
      // Project the region centre using the same pipeline as entity blips
      var rp = math.worldToRadar(
        region.x || 0, region.z || 0,
        shipX, shipZ, shipYaw, orientation
      );
      var rx = rp.rx - (self._panX || 0);
      var rz = rp.rz - (self._panZ || 0);
      var sp = math.radarToScreen(rx, rz, effectiveRange, R);
      var bx = cx + sp.sx;
      var by = cy + sp.sy;

      // Build CSS colour strings from [r, g, b] float array
      var c   = region.color || [0.6, 0.4, 1.0];
      var ri  = Math.round(c[0] * 255);
      var gi  = Math.round(c[1] * 255);
      var bi  = Math.round(c[2] * 255);
      var fillColor   = 'rgba(' + ri + ',' + gi + ',' + bi + ',0.3)';
      var strokeColor = 'rgb('  + ri + ',' + gi + ',' + bi + ')';

      switch (region.shape) {
        case 'sphere':
          self._drawRegionSphere(ctx, bx, by,
            region.radius || 0,
            scale, fillColor, strokeColor);
          break;
        case 'torus':
          self._drawRegionTorus(ctx, bx, by,
            region.outer_radius != null ? region.outer_radius : (region.radius || 0),
            region.inner_radius || 0,
            scale, strokeColor);
          break;
        case 'box': {
          var he = region.half_extents || [0, 0];
          self._drawRegionBox(ctx, bx, by,
            he[0] || 0, he[1] || 0,
            scale, fillColor, strokeColor);
          break;
        }
        // Unknown shape — skip (matches Bevy's `_ => None` in region_shape_from_snapshot)
      }

      // Optional name label — same style as entity blip labels
      if (region.name) {
        var labelOffset = Math.max(4,
          (region.radius || (region.half_extents ? (region.half_extents[0] || 0) : 0)) * scale
        ) + 4;
        ctx.font      = '11px "JetBrains Mono", monospace';
        ctx.fillStyle = 'rgba(153,255,217,0.9)';
        ctx.fillText(region.name, bx + labelOffset, by + 4);
      }
    });
  };

  /**
   * Sphere region: filled circle (Bevy: border-radius 50%, fill @ 0.3 alpha, stroke @ 1.0).
   *
   * Sizing (parity with Bevy `world_size_to_px`):
   *   radius_px = clamp(worldRadius * scale, 4, R)
   *   where scale = R / effectiveRange
   */
  RadarWidget.prototype._drawRegionSphere = function (ctx, bx, by, worldRadius, scale, fillColor, strokeColor) {
    var r_px = Math.max(4, Math.min(worldRadius * scale, scale > 0 ? 1e9 : 4));
    // Practical max = R (the whole radar) — no additional clamping needed since
    // anything bigger than R will just be off-screen or fill the disc.
    ctx.beginPath();
    ctx.arc(bx, by, r_px, 0, Math.PI * 2);
    ctx.fillStyle = fillColor;
    ctx.fill();
    ctx.strokeStyle = strokeColor;
    ctx.lineWidth   = 1.5;
    ctx.stroke();
  };

  /**
   * Torus region: annular ring (Bevy: transparent fill, full-alpha CSS border = ring width).
   *
   * Sizing:
   *   outerR_px = clamp(worldOuterR * scale, 4, ∞)  — Bevy uses world_size_to_px clamping
   *   innerR_px = max(0, worldInnerR * scale)        — Bevy: no world_size_to_px for inner
   *   ringCenter = (outerR + innerR) / 2
   *   ringWidth  = max(1, outerR - innerR)
   */
  RadarWidget.prototype._drawRegionTorus = function (ctx, bx, by, worldOuterR, worldInnerR, scale, strokeColor) {
    var outerR_px    = Math.max(4, worldOuterR * scale);
    var innerR_px    = Math.max(0, worldInnerR * scale);
    // Prevent inner from exceeding outer (degenerate data guard)
    if (innerR_px >= outerR_px) innerR_px = Math.max(0, outerR_px - 1);
    var ringCenter   = (outerR_px + innerR_px) / 2;
    var ringWidth    = Math.max(1, outerR_px - innerR_px);
    ctx.beginPath();
    ctx.arc(bx, by, ringCenter, 0, Math.PI * 2);
    ctx.lineWidth   = ringWidth;
    ctx.strokeStyle = strokeColor;
    ctx.stroke();
  };

  /**
   * Box region: axis-aligned filled rectangle (Bevy: yaw is present in data but
   * currently hardcoded to 0 and ignored in the renderer — we match that behaviour
   * for full visual parity).
   *
   * Sizing:
   *   halfW_px = max(4, halfExtentX * scale)
   *   halfH_px = max(4, halfExtentZ * scale)   (Z half-extent = height on radar)
   */
  RadarWidget.prototype._drawRegionBox = function (ctx, bx, by, worldHalfX, worldHalfZ, scale, fillColor, strokeColor) {
    var halfW_px = Math.max(4, worldHalfX * scale);
    var halfH_px = Math.max(4, worldHalfZ * scale);
    ctx.save();
    ctx.translate(bx, by);
    // NOTE: yaw rotation intentionally omitted — matches Bevy's axis-aligned rendering.
    // To add rotation in future: ctx.rotate(-(region.yaw || 0));
    ctx.fillStyle = fillColor;
    ctx.fillRect(-halfW_px, -halfH_px, halfW_px * 2, halfH_px * 2);
    ctx.strokeStyle = strokeColor;
    ctx.lineWidth   = 1.5;
    ctx.strokeRect(-halfW_px, -halfH_px, halfW_px * 2, halfH_px * 2);
    ctx.restore();
  };

  // ── Shared drawing helpers ────────────────────────────────────────────────

  /**
   * Draw a PNG icon blip at (bx, by) with radius dotR, applying a colour tint
   * from the entity's `color` field ([r, g, b] in 0–1 range).
   *
   * Tinting uses `source-atop` compositing: the icon is drawn first, then a
   * filled coloured rectangle is overlaid only where the icon has pixels,
   * at low opacity so the icon's own shading is still visible.
   */
  RadarWidget.prototype._drawIconBlip = function (ctx, icon, bx, by, dotR, color) {
    ctx.save();
    // Circular clip so the icon and tint stay within the blip circle
    ctx.beginPath();
    ctx.arc(bx, by, dotR, 0, Math.PI * 2);
    ctx.clip();

    var size = dotR * 2;
    ctx.drawImage(icon, bx - dotR, by - dotR, size, size);

    // Apply colour tint at 30% opacity using source-atop (paints only on
    // existing opaque pixels from the icon above)
    if (color && (color[0] !== 0 || color[1] !== 0 || color[2] !== 0)) {
      var r = Math.round(color[0] * 255);
      var g = Math.round(color[1] * 255);
      var b = Math.round(color[2] * 255);
      ctx.globalCompositeOperation = 'source-atop';
      ctx.globalAlpha = 0.3;
      ctx.fillStyle = 'rgb(' + r + ',' + g + ',' + b + ')';
      ctx.fillRect(bx - dotR, by - dotR, size, size);
      ctx.globalCompositeOperation = 'source-over';
      ctx.globalAlpha = 1.0;
    }

    ctx.restore();
  };

  /**
   * Draw a decorative ring around a blip.
   * @param {boolean} withTicks  draw 4 cross ticks (used for target lock ring)
   */
  RadarWidget.prototype._drawRing = function (ctx, bx, by, ringR, lineWidth, color, withTicks) {
    ctx.strokeStyle = color;
    ctx.lineWidth   = lineWidth;
    ctx.beginPath();
    ctx.arc(bx, by, ringR, 0, Math.PI * 2);
    ctx.stroke();

    if (withTicks) {
      ctx.lineWidth = 1.5;
      var lr = ringR;
      [[0, -lr - 4, 0, -lr + 4],
       [0,  lr - 4, 0,  lr + 4],
       [lr - 4, 0, lr + 4, 0],
       [-lr - 4, 0, -lr + 4, 0]].forEach(function (pts) {
        ctx.beginPath();
        ctx.moveTo(bx + pts[0], by + pts[1]);
        ctx.lineTo(bx + pts[2], by + pts[3]);
        ctx.stroke();
      });
    }
  };

  /** Own-ship marker — triangle pointing forward (up). */
  RadarWidget.prototype._drawOwnShip = function (ctx, cx, cy) {
    var icon = this._icons['player'];
    if (icon && icon.complete && icon.naturalWidth > 0) {
      var s = 18;
      ctx.drawImage(icon, cx - s / 2, cy - s / 2, s, s);
      return;
    }
    // Fallback: triangle
    ctx.fillStyle = '#6cb6d0';
    ctx.beginPath();
    ctx.moveTo(cx,     cy - 9);
    ctx.lineTo(cx + 5, cy + 3);
    ctx.lineTo(cx,     cy - 1);
    ctx.lineTo(cx - 5, cy + 3);
    ctx.closePath();
    ctx.fill();
  };

  // ── Icon loading (activated by Slice 2 / #446) ────────────────────────────

  /**
   * Pre-load all 7 blip icon PNGs in parallel.  Falls back gracefully when
   * the Image constructor is unavailable (e.g. Node.js test environment) or
   * when the PNG file does not exist (image.naturalWidth === 0 at render time).
   */
  RadarWidget.prototype._loadIcons = function () {
    var self = this;
    if (typeof Image === 'undefined') return;  // non-browser (test) environment
    Object.keys(ICON_STEMS).forEach(function (key) {
      var img = new Image();
      img.src = self._iconBasePath + ICON_STEMS[key] + '.png';
      self._icons[key] = img;
    });
  };

  // ── Hit testing ───────────────────────────────────────────────────────────

  /**
   * Return the blip (from _data) closest to (canvasX, canvasY) within hit
   * radius, or null.  canvasX/Y must be in canvas buffer pixels.
   */
  RadarWidget.prototype._getBlipAt = function (canvasX, canvasY) {
    var data = this._data;
    if (!data) return null;

    var W  = this._canvas.width, H = this._canvas.height;
    var cx = W / 2, cy = H / 2;
    var R  = Math.min(W, H) / 2 - 8;

    var best = null, bestDist = Infinity;

    if (data.mode === 'pre-projected') {
      var blips = data.blips || [];
      blips.forEach(function (b) {
        var bx   = cx + (b.radar_x != null ? b.radar_x * R : (b.sx || 0));
        var by   = cy - (b.radar_y != null ? b.radar_y * R : (b.sy || 0));
        var hitR = Math.max(14, (b.scaled_radius || 0) * R + 6);
        var dist = Math.hypot(canvasX - bx, canvasY - by);
        if (dist <= hitR && dist < bestDist) { best = b; bestDist = dist; }
      });
    } else if (data.mode === 'world-space' && this._projectedBlips) {
      // World-space blips already have absolute canvas coords (bx, by)
      this._projectedBlips.forEach(function (b) {
        var hitR = Math.max(14, b.dotR + 6);
        var dist = Math.hypot(canvasX - b.bx, canvasY - b.by);
        if (dist <= hitR && dist < bestDist) { best = b; bestDist = dist; }
      });
    }
    return best;
  };

  RadarWidget.prototype._onPointerTap = function (kind, e) {
    if (!this._onBlipTap || !this._canvas) return;
    // Suppress click that fires at the end of a drag gesture
    if (kind === 'click' && this._didDrag) { this._didDrag = false; return; }
    if (kind === 'touch') e.preventDefault();

    var rect  = this._canvas.getBoundingClientRect();
    var touch = e.touches ? e.touches[0] : e;
    var x     = touch.clientX - rect.left;
    var y     = touch.clientY - rect.top;
    // Scale CSS-pixel coords to canvas-buffer coords (handles DPR + CSS scaling)
    var scaleX = this._canvas.width  / rect.width;
    var scaleY = this._canvas.height / rect.height;
    var blip = this._getBlipAt(x * scaleX, y * scaleY);
    if (blip && blip.uuid) {
      this._onBlipTap(blip.uuid);
    }
  };

  // ── Gesture handlers (Slice 5b / #449) ────────────────────────────────────

  RadarWidget.prototype._onPointerDown = function (e) {
    if (!this._canvas) return;
    this._pointers[e.pointerId] = { x: e.clientX, y: e.clientY };
    if (this._canvas.setPointerCapture) {
      try { this._canvas.setPointerCapture(e.pointerId); } catch (_) {}
    }

    var ids = Object.keys(this._pointers);
    if (ids.length === 1) {
      // Begin drag — record start state and worldPerCss factor
      var rect  = this._canvas.getBoundingClientRect();
      var R_css = Math.max(1, Math.min(rect.width, rect.height) / 2);
      var data  = this._data;
      var range = (data && data.effective_range != null) ? data.effective_range : this._range;
      var effectiveRange = range / Math.max(this._zoom, 0.001);
      this._dragStart = {
        x:           e.clientX,
        y:           e.clientY,
        panX:        this._panX,
        panZ:        this._panZ,
        worldPerCss: effectiveRange / R_css,
      };
      this._didDrag = false;
    } else if (ids.length === 2) {
      // Cancel drag, begin pinch
      this._dragStart = null;
      var p0 = this._pointers[ids[0]], p1 = this._pointers[ids[1]];
      this._lastPinchDist = Math.hypot(p1.x - p0.x, p1.y - p0.y) || 1;
    }
  };

  RadarWidget.prototype._onPointerMove = function (e) {
    if (!this._pointers[e.pointerId]) return;
    this._pointers[e.pointerId] = { x: e.clientX, y: e.clientY };

    var ids = Object.keys(this._pointers);

    if (ids.length >= 2) {
      // Pinch zoom — use first two tracked pointers
      var p0 = this._pointers[ids[0]], p1 = this._pointers[ids[1]];
      var newDist = Math.hypot(p1.x - p0.x, p1.y - p0.y) || 1;
      if (this._lastPinchDist) {
        this.setZoom(this._zoom * (newDist / this._lastPinchDist));
        if (this._onZoomChange) this._onZoomChange(this._zoom);
      }
      this._lastPinchDist = newDist;
      this._dragStart = null;  // cancel drag while pinching
    } else if (ids.length === 1 && this._dragStart) {
      // Drag pan
      var dx = e.clientX - this._dragStart.x;
      var dy = e.clientY - this._dragStart.y;
      if (Math.abs(dx) > 3 || Math.abs(dy) > 3) this._didDrag = true;

      var wpc = this._dragStart.worldPerCss;
      this._panX = this._dragStart.panX - dx * wpc;
      this._panZ = this._dragStart.panZ + dy * wpc;  // canvas +Y is down = radar −Z = pan +Z
      if (this._onPanChange) this._onPanChange(this._panX, this._panZ);
    }
  };

  RadarWidget.prototype._onPointerUp = function (e) {
    delete this._pointers[e.pointerId];
    var ids = Object.keys(this._pointers);
    if (ids.length < 2) this._lastPinchDist = null;
    if (ids.length === 0) this._dragStart = null;
  };

  /** Double-tap / double-click — reset zoom and pan to defaults. */
  RadarWidget.prototype._onDblClick = function (e) {
    this._zoom = 1.0;
    this._panX = 0.0;
    this._panZ = 0.0;
    if (this._onZoomChange) this._onZoomChange(this._zoom);
    if (this._onPanChange)  this._onPanChange(this._panX, this._panZ);
  };

  return RadarWidget;
});
