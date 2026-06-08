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

    this._data      = null;
    this._rafId     = null;
    this._destroyed = false;

    // View state (zoom / pan) — populated by Slice 5b (#449)
    this._zoom = 1.0;
    this._panX = 0.0;
    this._panZ = 0.0;

    // Icon images pre-loaded in Slice 2 (#446)
    this._icons = {};

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

    // Pointer / touch event listeners
    this._boundClick    = this._onPointerTap.bind(this, 'click');
    this._boundTouchStart = this._onPointerTap.bind(this, 'touch');
    this._canvas.addEventListener('click',      this._boundClick);
    this._canvas.addEventListener('touchstart', this._boundTouchStart, { passive: false });
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

    this._data   = null;
    this._canvas = null;
    this._ctx    = null;
    this._icons  = {};
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

    ctx.clearRect(0, 0, W, H);

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
      var isObjective = objectiveUuids.indexOf(b.uuid) !== -1;
      var dotR = Math.max(MIN_BLIP_PX / 2, (b.scaled_radius || 0) * R * 0.6);

      // Objective gold ring (behind blip)
      if (isObjective) {
        self._drawRing(ctx, bx, by, 12, 2, '#d4a820', false);
      }

      // Try PNG icon; fall back to colored circle
      var icon = self._icons[b.icon || b.kind];
      if (icon && icon.complete && icon.naturalWidth > 0) {
        var iconSize = dotR * 2;
        ctx.drawImage(icon, bx - dotR, by - dotR, iconSize, iconSize);
      } else {
        var color = isTarget ? '#ff3344' : (KIND_COLOR[b.kind] || KIND_COLOR.unknown);
        ctx.beginPath();
        ctx.arc(bx, by, dotR, 0, Math.PI * 2);
        ctx.fillStyle = color;
        ctx.fill();
      }

      // Target highlight ring (red, on top)
      if (isTarget) {
        self._drawRing(ctx, bx, by, dotR + 7, 2, '#ff3344', true);
      }
    });
  };

  // ── World-space blip rendering (wired in Slice 5a / #447) ────────────────

  RadarWidget.prototype._drawWorldSpaceBlips = function (ctx, cx, cy, R, data) {
    // Projection is handled by radar-math.js imported by the console page.
    // The widget receives already-projected {sx, sy, dotR, ...} coords via
    // _projectWorldEntities() which is populated in Slice 5a (#447).
    if (typeof this._projectAndDrawWorldEntities === 'function') {
      this._projectAndDrawWorldEntities(ctx, cx, cy, R, data);
    }
  };

  // ── Shared drawing helpers ────────────────────────────────────────────────

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
   * Pre-load all 7 blip icon PNGs.  Called once, usually from constructor
   * after base setup is done (Slice 2 wires this in).
   */
  RadarWidget.prototype._loadIcons = function () {
    var self = this;
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

    var blips = null;
    if (data.mode === 'pre-projected') {
      blips = data.blips || [];
    } else if (data.mode === 'world-space' && this._projectedBlips) {
      blips = this._projectedBlips;
    }
    if (!blips) return null;

    var best = null, bestDist = Infinity;
    blips.forEach(function (b) {
      var bx   = cx + (b.radar_x != null ? b.radar_x * R : (b.sx || 0));
      var by   = cy - (b.radar_y != null ? b.radar_y * R : (b.sy || 0));
      var hitR = Math.max(14, (b.scaled_radius || 0) * R + 6);
      var dist = Math.hypot(canvasX - bx, canvasY - by);
      if (dist <= hitR && dist < bestDist) { best = b; bestDist = dist; }
    });
    return best;
  };

  RadarWidget.prototype._onPointerTap = function (kind, e) {
    if (!this._onBlipTap || !this._canvas) return;
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

  return RadarWidget;
});
