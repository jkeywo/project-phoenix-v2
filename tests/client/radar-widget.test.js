/**
 * tests/client/radar-widget.test.js
 *
 * Unit tests for gui/radar-widget.js — zoom/pan, gesture handlers,
 * auto-scale, and text-label features introduced in Slice 5b (#449).
 *
 * Runs in Node (vitest environment: 'node'), so we provide minimal fakes for:
 *   - requestAnimationFrame / cancelAnimationFrame
 *   - HTMLCanvasElement + CanvasRenderingContext2D
 *   - ResizeObserver, window  (absent → guarded by typeof checks in widget)
 */
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';

// ── Minimal canvas / context fakes ───────────────────────────────────────────

function makeCtx() {
  const drawn = { texts: [], arcs: [] };
  return {
    _drawn: drawn,
    clearRect: () => {},
    beginPath: () => {},
    arc:  (...a) => drawn.arcs.push(a),
    fill:  () => {},
    stroke: () => {},
    fillRect: () => {},
    moveTo: () => {},
    lineTo: () => {},
    closePath: () => {},
    save: () => {},
    restore: () => {},
    clip: () => {},
    drawImage: () => {},
    fillText: (text, x, y) => drawn.texts.push({ text, x, y }),
    measureText: () => ({ width: 50 }),
    // setters for style properties
    set fillStyle(_v) {},
    set strokeStyle(_v) {},
    set lineWidth(_v) {},
    set font(_v) {},
    set globalCompositeOperation(_v) {},
    set globalAlpha(_v) {},
  };
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
