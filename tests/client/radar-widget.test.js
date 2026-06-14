/**
 * tests/client/radar-widget.test.js
 *
 * Unit tests for gui/radar-widget.js — zoom/pan, gesture handlers,
 * auto-scale, text-label features (Slice 5b / #449), and region shape
 * rendering (PRD #443 parity with Bevy GenericRadarWidget).
 *
 * Runs in Node (vitest environment: 'node'), so we provide minimal fakes for:
 *   - requestAnimationFrame / cancelAnimationFrame
 *   - HTMLCanvasElement + CanvasRenderingContext2D
 *   - ResizeObserver, window  (absent → guarded by typeof checks in widget)
 */
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';

// ── Minimal canvas / context fakes ───────────────────────────────────────────

function makeCtx() {
  const drawn = { texts: [], arcs: [], fillRects: [], strokeRects: [], lineWidths: [], fillStyles: [], strokeStyles: [] };
  const ctx = {
    _drawn: drawn,
    clearRect: () => {},
    beginPath: () => {},
    arc:  (...a) => drawn.arcs.push(a),
    fill:  () => {},
    stroke: () => {},
    fillRect:   (...a) => drawn.fillRects.push(a),
    strokeRect: (...a) => drawn.strokeRects.push(a),
    moveTo: () => {},
    lineTo: () => {},
    closePath: () => {},
    save: () => {},
    restore: () => {},
    clip: () => {},
    translate: () => {},
    rotate: () => {},
    drawImage: () => {},
    fillText: (text, x, y) => drawn.texts.push({ text, x, y }),
    measureText: () => ({ width: 50 }),
  };
  // Track mutable style properties via backing store so tests can inspect them
  let _fillStyle = '', _strokeStyle = '', _lineWidth = 1, _font = '';
  let _gco = '', _ga = 1;
  Object.defineProperties(ctx, {
    fillStyle:   { get: () => _fillStyle,   set: v => { _fillStyle = v;   drawn.fillStyles.push(v);   }, enumerable: true },
    strokeStyle: { get: () => _strokeStyle, set: v => { _strokeStyle = v; drawn.strokeStyles.push(v); }, enumerable: true },
    lineWidth:   { get: () => _lineWidth,   set: v => { _lineWidth = v;   drawn.lineWidths.push(v);   }, enumerable: true },
    font:        { get: () => _font,        set: v => { _font = v; },         enumerable: true },
    globalCompositeOperation: { get: () => _gco, set: v => { _gco = v; }, enumerable: true },
    globalAlpha: { get: () => _ga,  set: v => { _ga = v; },  enumerable: true },
  });
  return ctx;
}

function makeCanvas(w, h) {
  w = w || 300; h = h || 300;
  const listeners = {};
  const ctx = makeCtx();
  const canvas = {
    nodeName: 'CANVAS',
    width:    w,
    height:   h,
    style:    {},
    _ctx:     ctx,
    getContext:            () => ctx,
    getBoundingClientRect: () => ({ left: 0, top: 0, width: w, height: h }),
    setPointerCapture:     () => {},
    addEventListener:    (ev, fn)       => { (listeners[ev] = listeners[ev] || []).push(fn); },
    removeEventListener: (ev, fn)       => { if (listeners[ev]) listeners[ev] = listeners[ev].filter(f => f !== fn); },
    _listeners: listeners,
    _fire:      (ev, data)              => (listeners[ev] || []).forEach(fn => fn(data)),
  };
  return canvas;
}

// ── rAF mocks (must be set before constructing RadarWidget) ──────────────────

let _rafId = 0;
function setupGlobals() {
  _rafId = 0;
  global.requestAnimationFrame = vi.fn(() => ++_rafId);
  global.cancelAnimationFrame  = vi.fn();
}
function teardownGlobals() {
  delete global.requestAnimationFrame;
  delete global.cancelAnimationFrame;
}

// ── Import radar-widget (UMD/IIFE exports {RadarWidget} via module.exports) ──
// CJS interop: vitest resolves the named export from module.exports.
import { RadarWidget } from '../../gui/radar-widget.js';

// ── Helper to create a widget with a fresh canvas ────────────────────────────

function makeWidget(opts) {
  setupGlobals();
  const canvas = makeCanvas();
  const widget = new RadarWidget(canvas, opts || {});
  return { widget, canvas };
}

// ─────────────────────────────────────────────────────────────────────────────
// zoom / pan API
// ─────────────────────────────────────────────────────────────────────────────

describe('RadarWidget: zoom / pan API', () => {
  afterEach(teardownGlobals);

  it('getZoom() returns 1.0 by default', () => {
    const { widget } = makeWidget();
    expect(widget.getZoom()).toBe(1.0);
    widget.destroy();
  });

  it('setZoom() / getZoom() round-trip', () => {
    const { widget } = makeWidget();
    widget.setZoom(2.5);
    expect(widget.getZoom()).toBe(2.5);
    widget.destroy();
  });

  it('setZoom() clamps to ZOOM_MAX (8.0)', () => {
    const { widget } = makeWidget();
    widget.setZoom(999);
    expect(widget.getZoom()).toBe(8.0);
    widget.destroy();
  });

  it('setZoom() clamps to ZOOM_MIN (0.25)', () => {
    const { widget } = makeWidget();
    widget.setZoom(0.001);
    expect(widget.getZoom()).toBe(0.25);
    widget.destroy();
  });

  it('getPan() returns {x:0, z:0} by default', () => {
    const { widget } = makeWidget();
    expect(widget.getPan()).toEqual({ x: 0, z: 0 });
    widget.destroy();
  });

  it('setPan() / getPan() round-trip', () => {
    const { widget } = makeWidget();
    widget.setPan(100, -200);
    expect(widget.getPan()).toEqual({ x: 100, z: -200 });
    widget.destroy();
  });
});

describe('RadarWidget: waypoint drawing', () => {
  afterEach(teardownGlobals);

  it('draws waypoint blips with the dedicated canvas shape', () => {
    const { widget, canvas } = makeWidget();
    const ctx = canvas._ctx;
    const arcsBefore = ctx._drawn.arcs.length;
    widget._drawPreProjectedBlips(ctx, 150, 150, 142, {
      mode: 'pre-projected',
      blips: [{ uuid: 'navigation-waypoint', radar_x: 0, radar_y: 0, scaled_radius: 0.02, kind: 'waypoint', edge: true }],
    });
    expect(ctx._drawn.arcs.length).toBeGreaterThan(arcsBefore);
    expect(ctx._drawn.strokeStyles).toContain('#72f3ff');
    widget.destroy();
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// double-tap reset
// ─────────────────────────────────────────────────────────────────────────────

describe('RadarWidget: double-tap reset', () => {
  afterEach(teardownGlobals);

  it('_onDblClick resets zoom to 1.0', () => {
    const { widget } = makeWidget();
    widget.setZoom(3.5);
    widget._onDblClick({});
    expect(widget.getZoom()).toBe(1.0);
    widget.destroy();
  });

  it('_onDblClick resets pan to (0, 0)', () => {
    const { widget } = makeWidget();
    widget.setPan(50, 80);
    widget._onDblClick({});
    expect(widget.getPan()).toEqual({ x: 0, z: 0 });
    widget.destroy();
  });

  it('_onDblClick fires onZoomChange(1.0)', () => {
    const onZoom = vi.fn();
    const { widget } = makeWidget({ onZoomChange: onZoom });
    widget.setZoom(2.0);
    widget._onDblClick({});
    expect(onZoom).toHaveBeenCalledWith(1.0);
    widget.destroy();
  });

  it('_onDblClick fires onPanChange(0, 0)', () => {
    const onPan = vi.fn();
    const { widget } = makeWidget({ onPanChange: onPan });
    widget.setPan(10, 20);
    widget._onDblClick({});
    expect(onPan).toHaveBeenCalledWith(0, 0);
    widget.destroy();
  });

  it('_onDblClick is idempotent — no error if called twice', () => {
    const { widget } = makeWidget();
    widget._onDblClick({});
    widget._onDblClick({});
    expect(widget.getZoom()).toBe(1.0);
    widget.destroy();
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// pointer gesture state
// ─────────────────────────────────────────────────────────────────────────────

describe('RadarWidget: pointer tracking', () => {
  afterEach(teardownGlobals);

  it('_onPointerDown registers pointer id', () => {
    const { widget } = makeWidget();
    widget._onPointerDown({ pointerId: 1, clientX: 100, clientY: 100 });
    expect(Object.keys(widget._pointers).length).toBe(1);
    widget.destroy();
  });

  it('_onPointerUp removes pointer id', () => {
    const { widget } = makeWidget();
    widget._onPointerDown({ pointerId: 1, clientX: 100, clientY: 100 });
    widget._onPointerUp({ pointerId: 1 });
    expect(Object.keys(widget._pointers).length).toBe(0);
    widget.destroy();
  });

  it('_onPointerDown with two pointers sets _lastPinchDist', () => {
    const { widget } = makeWidget();
    widget._onPointerDown({ pointerId: 1, clientX: 100, clientY: 150 });
    widget._onPointerDown({ pointerId: 2, clientX: 200, clientY: 150 });
    // distance = 100
    expect(widget._lastPinchDist).toBeCloseTo(100);
    widget.destroy();
  });

  it('_onPointerUp with < 2 remaining pointers clears _lastPinchDist', () => {
    const { widget } = makeWidget();
    widget._onPointerDown({ pointerId: 1, clientX: 0,   clientY: 0 });
    widget._onPointerDown({ pointerId: 2, clientX: 100, clientY: 0 });
    widget._onPointerUp({ pointerId: 2 });
    expect(widget._lastPinchDist).toBeNull();
    widget.destroy();
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// drag-pan
// ─────────────────────────────────────────────────────────────────────────────

describe('RadarWidget: drag-pan', () => {
  afterEach(teardownGlobals);

  it('dragging right decreases panX (entities shift left in view)', () => {
    const { widget } = makeWidget();
    widget.setZoom(1.0);
    widget._onPointerDown({ pointerId: 1, clientX: 100, clientY: 100 });
    widget._onPointerMove({ pointerId: 1, clientX: 150, clientY: 100 });  // +50 px right
    expect(widget.getPan().x).toBeLessThan(0);
    widget.destroy();
  });

  it('dragging left increases panX', () => {
    const { widget } = makeWidget();
    widget._onPointerDown({ pointerId: 1, clientX: 150, clientY: 100 });
    widget._onPointerMove({ pointerId: 1, clientX: 100, clientY: 100 });  // −50 px left
    expect(widget.getPan().x).toBeGreaterThan(0);
    widget.destroy();
  });

  it('dragging down increases panZ (canvas +Y = down = radar −Z inverted)', () => {
    const { widget } = makeWidget();
    widget._onPointerDown({ pointerId: 1, clientX: 100, clientY: 100 });
    widget._onPointerMove({ pointerId: 1, clientX: 100, clientY: 150 });  // +50 px down
    expect(widget.getPan().z).toBeGreaterThan(0);
    widget.destroy();
  });

  it('small movement (≤3 px) does NOT set _didDrag', () => {
    const { widget } = makeWidget();
    widget._onPointerDown({ pointerId: 1, clientX: 100, clientY: 100 });
    widget._onPointerMove({ pointerId: 1, clientX: 101, clientY: 100 });  // 1 px
    expect(widget._didDrag).toBe(false);
    widget.destroy();
  });

  it('large movement (>3 px) sets _didDrag', () => {
    const { widget } = makeWidget();
    widget._onPointerDown({ pointerId: 1, clientX: 100, clientY: 100 });
    widget._onPointerMove({ pointerId: 1, clientX: 110, clientY: 100 });  // 10 px
    expect(widget._didDrag).toBe(true);
    widget.destroy();
  });

  it('drag fires onPanChange callback', () => {
    const onPan = vi.fn();
    const { widget } = makeWidget({ onPanChange: onPan });
    widget._onPointerDown({ pointerId: 1, clientX: 0, clientY: 0 });
    widget._onPointerMove({ pointerId: 1, clientX: 50, clientY: 0 });
    expect(onPan).toHaveBeenCalled();
    widget.destroy();
  });

  it('pan delta is proportional to drag distance', () => {
    const { widget } = makeWidget();
    widget.setZoom(1.0);
    widget._onPointerDown({ pointerId: 1, clientX: 0, clientY: 0 });
    widget._onPointerMove({ pointerId: 1, clientX: 10, clientY: 0 });
    const pan1 = widget.getPan().x;
    widget._onDblClick({});  // reset
    widget._onPointerDown({ pointerId: 1, clientX: 0, clientY: 0 });
    widget._onPointerMove({ pointerId: 1, clientX: 20, clientY: 0 });
    const pan2 = widget.getPan().x;
    // Moving twice as far should produce twice the pan delta (within tolerance)
    expect(Math.abs(pan2)).toBeCloseTo(Math.abs(pan1) * 2, 1);
    widget.destroy();
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// pinch-zoom
// ─────────────────────────────────────────────────────────────────────────────

describe('RadarWidget: pinch-zoom', () => {
  afterEach(teardownGlobals);

  it('spreading fingers increases zoom', () => {
    const { widget } = makeWidget();
    // Initial fingers 100 px apart
    widget._onPointerDown({ pointerId: 1, clientX: 100, clientY: 150 });
    widget._onPointerDown({ pointerId: 2, clientX: 200, clientY: 150 });
    // Move fingers to 200 px apart
    widget._pointers[2] = { x: 250, y: 150 };
    widget._onPointerMove({ pointerId: 2, clientX: 250, clientY: 150 });
    expect(widget.getZoom()).toBeGreaterThan(1.0);
    widget.destroy();
  });

  it('pinching fingers decreases zoom', () => {
    const { widget } = makeWidget();
    widget.setZoom(2.0);
    // Initial fingers 200 px apart
    widget._onPointerDown({ pointerId: 1, clientX: 50,  clientY: 150 });
    widget._onPointerDown({ pointerId: 2, clientX: 250, clientY: 150 });
    // Move fingers to 100 px apart
    widget._pointers[1] = { x: 100, y: 150 };
    widget._pointers[2] = { x: 200, y: 150 };
    widget._onPointerMove({ pointerId: 2, clientX: 200, clientY: 150 });
    expect(widget.getZoom()).toBeLessThan(2.0);
    widget.destroy();
  });

  it('zoom is clamped at ZOOM_MAX (8.0) during pinch', () => {
    const { widget } = makeWidget();
    widget.setZoom(7.9);
    widget._onPointerDown({ pointerId: 1, clientX: 100, clientY: 150 });
    widget._onPointerDown({ pointerId: 2, clientX: 101, clientY: 150 });  // 1 px apart
    // Spread to 1000 px — huge zoom request
    widget._pointers[2] = { x: 1101, y: 150 };
    widget._onPointerMove({ pointerId: 2, clientX: 1101, clientY: 150 });
    expect(widget.getZoom()).toBe(8.0);
    widget.destroy();
  });

  it('zoom is clamped at ZOOM_MIN (0.25) during pinch', () => {
    const { widget } = makeWidget();
    widget.setZoom(0.26);
    widget._onPointerDown({ pointerId: 1, clientX: 0,    clientY: 150 });
    widget._onPointerDown({ pointerId: 2, clientX: 1000, clientY: 150 });  // 1000 px apart
    // Pinch to 1 px
    widget._pointers[2] = { x: 1, y: 150 };
    widget._onPointerMove({ pointerId: 2, clientX: 1, clientY: 150 });
    expect(widget.getZoom()).toBe(0.25);
    widget.destroy();
  });

  it('pinch fires onZoomChange', () => {
    const onZoom = vi.fn();
    const { widget } = makeWidget({ onZoomChange: onZoom });
    widget._onPointerDown({ pointerId: 1, clientX: 100, clientY: 150 });
    widget._onPointerDown({ pointerId: 2, clientX: 200, clientY: 150 });
    widget._pointers[2] = { x: 300, y: 150 };
    widget._onPointerMove({ pointerId: 2, clientX: 300, clientY: 150 });
    expect(onZoom).toHaveBeenCalled();
    widget.destroy();
  });

  it('move with untracked pointerId is silently ignored', () => {
    const { widget } = makeWidget();
    // No pointerDown for id 99 — should not throw
    expect(() => widget._onPointerMove({ pointerId: 99, clientX: 0, clientY: 0 })).not.toThrow();
    widget.destroy();
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// destroy cleanup
// ─────────────────────────────────────────────────────────────────────────────

describe('RadarWidget: destroy', () => {
  afterEach(teardownGlobals);

  it('destroy() calls cancelAnimationFrame', () => {
    const { widget } = makeWidget();
    widget.destroy();
    expect(global.cancelAnimationFrame).toHaveBeenCalled();
  });

  it('destroy() nulls canvas reference', () => {
    const { widget } = makeWidget();
    widget.destroy();
    expect(widget._canvas).toBeNull();
  });

  it('calling destroy() twice does not throw', () => {
    const { widget } = makeWidget();
    expect(() => { widget.destroy(); widget.destroy(); }).not.toThrow();
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// _getBlipAt hit-testing — pre-projected and world-space modes
// ─────────────────────────────────────────────────────────────────────────────

describe('RadarWidget: _getBlipAt', () => {
  afterEach(teardownGlobals);

  // ── Pre-projected mode ────────────────────────────────────────────────────

  it('returns null when no data is set', () => {
    const { widget } = makeWidget();
    expect(widget._getBlipAt(150, 150)).toBeNull();
    widget.destroy();
  });

  it('pre-projected: returns blip when clicking at its canvas position (centre)', () => {
    const { widget } = makeWidget({ onBlipTap: () => {} });
    // radar_x=0, radar_y=0 → canvas centre (150, 150) for a 300×300 canvas
    widget.update({
      mode: 'pre-projected',
      blips: [{ uuid: 'abc', radar_x: 0, radar_y: 0, scaled_radius: 0.02, kind: 'ship' }],
    });
    const blip = widget._getBlipAt(150, 150);
    expect(blip).not.toBeNull();
    expect(blip.uuid).toBe('abc');
    widget.destroy();
  });

  it('pre-projected: returns null when clicking away from all blips', () => {
    const { widget } = makeWidget();
    widget.update({
      mode: 'pre-projected',
      blips: [{ uuid: 'abc', radar_x: 0, radar_y: 0, scaled_radius: 0.01, kind: 'ship' }],
    });
    // Click far from centre (top-left corner — canvas (0, 0) vs blip at (150, 150))
    const blip = widget._getBlipAt(0, 0);
    expect(blip).toBeNull();
    widget.destroy();
  });

  it('pre-projected: returns closest blip among multiple blips', () => {
    const { widget } = makeWidget();
    widget.update({
      mode: 'pre-projected',
      blips: [
        { uuid: 'far',   radar_x:  0.5, radar_y: 0, scaled_radius: 0.01, kind: 'ship' },
        { uuid: 'close', radar_x: -0.5, radar_y: 0, scaled_radius: 0.01, kind: 'ship' },
      ],
    });
    // canvas width=height=300, centre=150, R≈142.
    // 'close' blip is at x = 150 + (-0.5)*142 ≈ 79, y = 150.
    // Clicking at (79, 150) should return 'close'.
    const blip = widget._getBlipAt(79, 150);
    expect(blip).not.toBeNull();
    expect(blip.uuid).toBe('close');
    widget.destroy();
  });

  it('pre-projected: waypoint blips remain hit-testable', () => {
    const { widget } = makeWidget();
    widget.update({
      mode: 'pre-projected',
      blips: [{ uuid: 'navigation-waypoint', radar_x: 0, radar_y: 0, scaled_radius: 0.02, kind: 'waypoint', edge: true }],
    });
    const blip = widget._getBlipAt(150, 150);
    expect(blip).not.toBeNull();
    expect(blip.uuid).toBe('navigation-waypoint');
    widget.destroy();
  });

  // ── World-space mode ──────────────────────────────────────────────────────

  it('world-space: returns null when _projectedBlips is null/empty', () => {
    const { widget } = makeWidget();
    widget.update({ mode: 'world-space', entities: [] });
    // _projectedBlips is null until first render — update() only stores data, doesn't render
    expect(widget._getBlipAt(150, 150)).toBeNull();
    widget.destroy();
  });

  it('world-space: returns blip by stored canvas coords', () => {
    const { widget } = makeWidget();
    // Directly inject a projected blip (as _projectAndDrawWorldEntities would produce)
    widget._projectedBlips = [{ uuid: 'ws-1', bx: 150, by: 150, dotR: 8 }];
    widget._data = { mode: 'world-space' };
    const blip = widget._getBlipAt(150, 150);
    expect(blip).not.toBeNull();
    expect(blip.uuid).toBe('ws-1');
    widget.destroy();
  });

  it('world-space: returns null for click far from blip', () => {
    const { widget } = makeWidget();
    widget._projectedBlips = [{ uuid: 'ws-1', bx: 150, by: 150, dotR: 8 }];
    widget._data = { mode: 'world-space' };
    // click at canvas (0, 0) — far from centre
    expect(widget._getBlipAt(0, 0)).toBeNull();
    widget.destroy();
  });

  it('world-space: uses hitR = max(14, dotR+6), so small blips still have 14px minimum', () => {
    const { widget } = makeWidget();
    // Tiny blip dotR=2, hitR=max(14, 8)=14. Click 12px away — should still hit.
    widget._projectedBlips = [{ uuid: 'ws-tiny', bx: 150, by: 150, dotR: 2 }];
    widget._data = { mode: 'world-space' };
    const blip = widget._getBlipAt(150 + 12, 150);  // 12 px right of centre
    expect(blip).not.toBeNull();
    expect(blip.uuid).toBe('ws-tiny');
    widget.destroy();
  });

  it('world-space: picks closest among overlapping projected blips', () => {
    const { widget } = makeWidget();
    widget._projectedBlips = [
      { uuid: 'a', bx: 150, by: 150, dotR: 10 },
      { uuid: 'b', bx: 155, by: 150, dotR: 10 },  // 5 px to the right
    ];
    widget._data = { mode: 'world-space' };
    // Click at (156, 150) — closer to 'b'
    const blip = widget._getBlipAt(156, 150);
    expect(blip).not.toBeNull();
    expect(blip.uuid).toBe('b');
    widget.destroy();
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// Region shape rendering — _drawWorldSpaceRegions, _drawRegionSphere,
// _drawRegionTorus, _drawRegionBox
// ─────────────────────────────────────────────────────────────────────────────

// Minimal RadarMath stub for injection into widgets that need world-space rendering
const MATH_STUB = {
  worldToRadar:  (ex, ez, sx, sz, yaw, ori) => ({ rx: ex - sx, rz: sz - ez }),
  radarToScreen: (rx, rz, range, R) => ({ sx: rx * R / range, sy: -rz * R / range }),
  autoScaleRange: (pts) => 300,
};

function makeWidgetWithMath(opts) {
  setupGlobals();
  const canvas = makeCanvas(300, 300);
  const widget = new RadarWidget(canvas, Object.assign({ math: MATH_STUB }, opts || {}));
  return { widget, canvas, ctx: canvas._ctx };
}

describe('RadarWidget: _drawRegionSphere', () => {
  afterEach(teardownGlobals);

  it('draws an arc for a sphere region', () => {
    const { widget, ctx } = makeWidgetWithMath();
    const arcsBefore = ctx._drawn.arcs.length;
    widget._drawRegionSphere(ctx, 150, 150, 50, 1.42, 'rgba(255,0,0,0.3)', 'rgb(255,0,0)');
    // Should have called arc at least once
    expect(ctx._drawn.arcs.length).toBeGreaterThan(arcsBefore);
    widget.destroy();
  });

  it('uses a minimum radius of 4px', () => {
    const { widget, ctx } = makeWidgetWithMath();
    ctx._drawn.arcs = [];
    // worldRadius=0.001 * scale=1 = 0.001 < 4 → clamped to 4
    widget._drawRegionSphere(ctx, 150, 150, 0.001, 1, 'rgba(0,0,0,0.3)', 'rgb(0,0,0)');
    const lastArc = ctx._drawn.arcs[ctx._drawn.arcs.length - 1];
    expect(lastArc[2]).toBe(4);  // radius arg (index 2)
    widget.destroy();
  });

  it('scales radius correctly: worldRadius * scale', () => {
    const { widget, ctx } = makeWidgetWithMath();
    ctx._drawn.arcs = [];
    // worldRadius=50, scale=1.0 → 50px; above minimum of 4
    widget._drawRegionSphere(ctx, 150, 150, 50, 1.0, 'rgba(0,0,0,0.3)', 'rgb(0,0,0)');
    const lastArc = ctx._drawn.arcs[ctx._drawn.arcs.length - 1];
    expect(lastArc[2]).toBeCloseTo(50);
    widget.destroy();
  });

  it('draws fill at 0.3 alpha and stroke at full alpha', () => {
    const { widget, ctx } = makeWidgetWithMath();
    ctx._drawn.fillStyles = [];
    ctx._drawn.strokeStyles = [];
    widget._drawRegionSphere(ctx, 0, 0, 20, 1, 'rgba(200,100,50,0.3)', 'rgb(200,100,50)');
    expect(ctx._drawn.fillStyles.some(s => s.includes('0.3'))).toBe(true);
    expect(ctx._drawn.strokeStyles.some(s => s.startsWith('rgb('))).toBe(true);
    widget.destroy();
  });
});

describe('RadarWidget: _drawRegionTorus', () => {
  afterEach(teardownGlobals);

  it('draws an arc for the ring', () => {
    const { widget, ctx } = makeWidgetWithMath();
    ctx._drawn.arcs = [];
    widget._drawRegionTorus(ctx, 150, 150, 60, 30, 1.0, 'rgb(100,150,200)');
    expect(ctx._drawn.arcs.length).toBeGreaterThan(0);
    widget.destroy();
  });

  it('ring centre is at (outerR + innerR) / 2', () => {
    const { widget, ctx } = makeWidgetWithMath();
    ctx._drawn.arcs = [];
    // outerR=60, innerR=30 → ringCenter=45
    widget._drawRegionTorus(ctx, 150, 150, 60, 30, 1.0, 'rgb(0,0,0)');
    const arc = ctx._drawn.arcs[0];
    expect(arc[2]).toBeCloseTo(45);  // radius = ringCenter
    widget.destroy();
  });

  it('lineWidth = outerR - innerR', () => {
    const { widget, ctx } = makeWidgetWithMath();
    ctx._drawn.lineWidths = [];
    // outerR=60, innerR=30 → ringWidth=30
    widget._drawRegionTorus(ctx, 150, 150, 60, 30, 1.0, 'rgb(0,0,0)');
    expect(ctx._drawn.lineWidths).toContain(30);
    widget.destroy();
  });

  it('clamps minimum ring width to 1px', () => {
    const { widget, ctx } = makeWidgetWithMath();
    ctx._drawn.lineWidths = [];
    // outerR=5, innerR=4.9 → ringWidth=0.1 → clamped to 1
    widget._drawRegionTorus(ctx, 150, 150, 5, 4.9, 1.0, 'rgb(0,0,0)');
    const minLw = Math.min(...ctx._drawn.lineWidths.filter(v => v > 0));
    expect(minLw).toBeGreaterThanOrEqual(1);
    widget.destroy();
  });

  it('handles degenerate torus where inner >= outer without throwing', () => {
    const { widget, ctx } = makeWidgetWithMath();
    expect(() => widget._drawRegionTorus(ctx, 150, 150, 10, 50, 1.0, 'rgb(0,0,0)')).not.toThrow();
    widget.destroy();
  });

  it('uses no fill (no fillRect / no fill style applied for torus)', () => {
    const { widget, ctx } = makeWidgetWithMath();
    ctx._drawn.fillRects = [];
    ctx._drawn.fillStyles = [];
    widget._drawRegionTorus(ctx, 150, 150, 60, 20, 1.0, 'rgb(0,0,0)');
    // Torus is stroke-only — no fillRect and fillStyle should not be set
    expect(ctx._drawn.fillRects).toHaveLength(0);
    // fillStyle should not have been set by torus (Bevy: Color::NONE fill)
    expect(ctx._drawn.fillStyles).toHaveLength(0);
    widget.destroy();
  });
});

describe('RadarWidget: _drawRegionBox', () => {
  afterEach(teardownGlobals);

  it('calls fillRect and strokeRect', () => {
    const { widget, ctx } = makeWidgetWithMath();
    ctx._drawn.fillRects = [];
    ctx._drawn.strokeRects = [];
    widget._drawRegionBox(ctx, 150, 150, 40, 30, 1.0, 'rgba(0,255,0,0.3)', 'rgb(0,255,0)');
    expect(ctx._drawn.fillRects.length).toBeGreaterThan(0);
    expect(ctx._drawn.strokeRects.length).toBeGreaterThan(0);
    widget.destroy();
  });

  it('rect is centred: fillRect args are (-halfW, -halfH, 2*halfW, 2*halfH)', () => {
    const { widget, ctx } = makeWidgetWithMath();
    ctx._drawn.fillRects = [];
    // worldHalfX=40, worldHalfZ=30, scale=1 → halfW=40, halfH=30
    widget._drawRegionBox(ctx, 150, 150, 40, 30, 1.0, 'rgba(0,0,0,0.3)', 'rgb(0,0,0)');
    const fr = ctx._drawn.fillRects[0];
    expect(fr[0]).toBeCloseTo(-40);   // x
    expect(fr[1]).toBeCloseTo(-30);   // y
    expect(fr[2]).toBeCloseTo(80);    // width = 2*halfW
    expect(fr[3]).toBeCloseTo(60);    // height = 2*halfH
    widget.destroy();
  });

  it('scales half-extents by scale factor', () => {
    const { widget, ctx } = makeWidgetWithMath();
    ctx._drawn.fillRects = [];
    // worldHalfX=10, scale=3.0 → halfW=30
    widget._drawRegionBox(ctx, 150, 150, 10, 5, 3.0, 'rgba(0,0,0,0.3)', 'rgb(0,0,0)');
    const fr = ctx._drawn.fillRects[0];
    expect(fr[0]).toBeCloseTo(-30);   // -halfW = -10*3
    expect(fr[1]).toBeCloseTo(-15);   // -halfH = -5*3
    widget.destroy();
  });

  it('uses a minimum half-size of 4px', () => {
    const { widget, ctx } = makeWidgetWithMath();
    ctx._drawn.fillRects = [];
    // worldHalfX=0.001, scale=1 → halfW=0.001 < 4 → clamped to 4
    widget._drawRegionBox(ctx, 150, 150, 0.001, 0.001, 1, 'rgba(0,0,0,0.3)', 'rgb(0,0,0)');
    const fr = ctx._drawn.fillRects[0];
    expect(fr[0]).toBeCloseTo(-4);
    widget.destroy();
  });

  it('draws fill at 0.3 alpha', () => {
    const { widget, ctx } = makeWidgetWithMath();
    ctx._drawn.fillStyles = [];
    widget._drawRegionBox(ctx, 0, 0, 20, 20, 1, 'rgba(255,128,0,0.3)', 'rgb(255,128,0)');
    expect(ctx._drawn.fillStyles.some(s => s.includes('0.3'))).toBe(true);
    widget.destroy();
  });
});

describe('RadarWidget: _drawWorldSpaceRegions projection', () => {
  afterEach(teardownGlobals);

  it('does nothing when regions array is empty', () => {
    const { widget, ctx } = makeWidgetWithMath();
    ctx._drawn.arcs = [];
    ctx._drawn.fillRects = [];
    widget._drawWorldSpaceRegions(ctx, 150, 150, 142, [], 0, 0, 0, 'ship_relative', 300);
    expect(ctx._drawn.arcs).toHaveLength(0);
    expect(ctx._drawn.fillRects).toHaveLength(0);
    widget.destroy();
  });

  it('does nothing when math is not set', () => {
    setupGlobals();
    const canvas = makeCanvas(300, 300);
    const widget = new RadarWidget(canvas, {});  // no math injected, no window.RadarMath
    const ctx = canvas._ctx;
    ctx._drawn.arcs = [];
    widget._drawWorldSpaceRegions(ctx, 150, 150, 142, [
      { uuid: 'r1', x: 0, z: 0, shape: 'sphere', radius: 50, color: [1, 0, 0] },
    ], 0, 0, 0, 'ship_relative', 300);
    expect(ctx._drawn.arcs).toHaveLength(0);
    widget.destroy();
  });

  it('sphere: draws arc at projected position', () => {
    const { widget, ctx } = makeWidgetWithMath();
    ctx._drawn.arcs = [];
    // Ship at origin, entity at (10, 0) → projected offset
    widget._drawWorldSpaceRegions(ctx, 150, 150, 142, [
      { uuid: 'r1', x: 10, z: 0, shape: 'sphere', radius: 20, color: [1, 0, 0] },
    ], 0, 0, 0, 'ship_relative', 100);
    const arc = ctx._drawn.arcs.find(a => a[2] > 4);  // find the sphere arc
    expect(arc).toBeDefined();
    // x-offset should be non-zero (entity is to starboard)
    // arc[0] = cx + sx where sx = (10-0)*142/100 = 14.2, so arc[0] ≈ 150+14.2 = 164.2
    expect(arc[0]).toBeGreaterThan(150);
    widget.destroy();
  });

  it('torus: draws ring at projected position', () => {
    const { widget, ctx } = makeWidgetWithMath();
    ctx._drawn.arcs = [];
    widget._drawWorldSpaceRegions(ctx, 150, 150, 142, [
      { uuid: 'r2', x: 0, z: 0, shape: 'torus', radius: 80, inner_radius: 40, outer_radius: 80, color: [0, 1, 0] },
    ], 0, 0, 0, 'ship_relative', 200);
    expect(ctx._drawn.arcs.length).toBeGreaterThan(0);
    widget.destroy();
  });

  it('box: draws fillRect at projected position', () => {
    const { widget, ctx } = makeWidgetWithMath();
    ctx._drawn.fillRects = [];
    widget._drawWorldSpaceRegions(ctx, 150, 150, 142, [
      { uuid: 'r3', x: 0, z: 0, shape: 'box', half_extents: [30, 20], color: [0, 0, 1] },
    ], 0, 0, 0, 'ship_relative', 100);
    expect(ctx._drawn.fillRects.length).toBeGreaterThan(0);
    widget.destroy();
  });

  it('unknown shape is silently skipped', () => {
    const { widget, ctx } = makeWidgetWithMath();
    ctx._drawn.arcs = [];
    ctx._drawn.fillRects = [];
    expect(() => widget._drawWorldSpaceRegions(ctx, 150, 150, 142, [
      { uuid: 'r4', x: 0, z: 0, shape: 'cylinder', radius: 10, color: [1, 1, 0] },
    ], 0, 0, 0, 'ship_relative', 100)).not.toThrow();
    expect(ctx._drawn.arcs).toHaveLength(0);
    expect(ctx._drawn.fillRects).toHaveLength(0);
    widget.destroy();
  });

  it('renders region name label when name is set', () => {
    const { widget, ctx } = makeWidgetWithMath();
    ctx._drawn.texts = [];
    widget._drawWorldSpaceRegions(ctx, 150, 150, 142, [
      { uuid: 'r5', x: 0, z: 0, shape: 'sphere', radius: 30, color: [1, 0, 0], name: 'Danger Zone' },
    ], 0, 0, 0, 'ship_relative', 100);
    const label = ctx._drawn.texts.find(t => t.text === 'Danger Zone');
    expect(label).toBeDefined();
    widget.destroy();
  });

  it('does not render label when name is absent', () => {
    const { widget, ctx } = makeWidgetWithMath();
    ctx._drawn.texts = [];
    widget._drawWorldSpaceRegions(ctx, 150, 150, 142, [
      { uuid: 'r6', x: 0, z: 0, shape: 'sphere', radius: 30, color: [1, 0, 0] },
    ], 0, 0, 0, 'ship_relative', 100);
    expect(ctx._drawn.texts).toHaveLength(0);
    widget.destroy();
  });

  it('pan offset is applied to region positions', () => {
    const { widget, ctx } = makeWidgetWithMath();
    // Without pan
    ctx._drawn.arcs = [];
    widget.setPan(0, 0);
    widget._drawWorldSpaceRegions(ctx, 150, 150, 142, [
      { uuid: 'r7', x: 50, z: 0, shape: 'sphere', radius: 5, color: [1, 1, 1] },
    ], 0, 0, 0, 'ship_relative', 100);
    const arcNoPan = ctx._drawn.arcs[0];

    // With pan (shift region left by 50 world units → back to centre)
    ctx._drawn.arcs = [];
    widget.setPan(50, 0);
    widget._drawWorldSpaceRegions(ctx, 150, 150, 142, [
      { uuid: 'r7', x: 50, z: 0, shape: 'sphere', radius: 5, color: [1, 1, 1] },
    ], 0, 0, 0, 'ship_relative', 100);
    const arcWithPan = ctx._drawn.arcs[0];

    // With pan=50, region at x=50 lands at same position as region at x=0 with pan=0
    expect(arcWithPan[0]).toBeCloseTo(150, 0);
    expect(arcNoPan[0]).toBeGreaterThan(150);
    widget.destroy();
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// Render-on-demand (impulse-charge flicker fix)
// ─────────────────────────────────────────────────────────────────────────────

describe('RadarWidget: render-on-demand', () => {
  afterEach(teardownGlobals);

  it('rAF loop does not repaint when nothing changed', () => {
    const { widget } = makeWidget();
    widget.update({ mode: 'pre-projected', blips: [] }); // clears the dirty flag
    const spy = vi.spyOn(widget, '_render');
    widget._loop(); // a frame with no intervening change
    widget._loop();
    expect(spy).not.toHaveBeenCalled();
    widget.destroy();
  });

  it('rAF loop repaints after update() marks the canvas dirty', () => {
    const { widget } = makeWidget();
    widget.update({ mode: 'pre-projected', blips: [] });
    widget._loop(); // drains any pending render
    const spy = vi.spyOn(widget, '_render');
    widget.update({ mode: 'pre-projected', blips: [{ uuid: 'a', radar_x: 0, radar_y: 0, kind: 'ship' }] });
    expect(spy).toHaveBeenCalledTimes(1); // update() renders immediately
    spy.mockClear();
    widget._loop(); // dirty already cleared by the immediate render
    expect(spy).not.toHaveBeenCalled();
    widget.destroy();
  });

  it('pan/zoom gestures mark the canvas dirty for the next frame', () => {
    const { widget } = makeWidget();
    widget.update({ mode: 'pre-projected', blips: [] });
    widget._loop();
    const spy = vi.spyOn(widget, '_render');
    widget.setZoom(2.0);
    widget._loop();
    expect(spy).toHaveBeenCalled();
    widget.destroy();
  });
});
