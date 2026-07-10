// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import '../../gui/components/ph-navigation-map.js';

function makeFakeCtx() {
  const calls = { fillRect: [], arc: [], fill: [], fillText: [], moveTo: [], lineTo: [], stroke: [], beginPath: [] };
  const ctx = {
    _calls: calls,
    fillStyle: '',
    strokeStyle: '',
    lineWidth: 0,
    font: '',
    textAlign: 'left',
    shadowColor: '',
    shadowBlur: 0,
    fillRect: (...a) => calls.fillRect.push(a),
    beginPath: () => { calls.beginPath.push(true); },
    arc: (...a) => calls.arc.push(a),
    fill: () => calls.fill.push(true),
    stroke: () => calls.stroke.push(true),
    moveTo: (...a) => calls.moveTo.push(a),
    lineTo: (...a) => calls.lineTo.push(a),
    fillText: (...a) => calls.fillText.push(a),
    save: vi.fn(),
    restore: vi.fn(),
    closePath: vi.fn(),
    translate: vi.fn(),
    rotate: vi.fn(),
    drawImage: vi.fn(),
    setLineDash: vi.fn(),
  };
  return ctx;
}

let fakeCtx;
let origGetContext;
let origRAF;
let origCARAF;
let origRO;
let roCallback;

beforeEach(() => {
  fakeCtx = makeFakeCtx();

  origGetContext = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = function () { return fakeCtx; };

  let rafCb = null;
  origRAF = window.requestAnimationFrame;
  window.requestAnimationFrame = vi.fn((cb) => { rafCb = cb; return 1; });
  origCARAF = window.cancelAnimationFrame;
  window.cancelAnimationFrame = vi.fn();

  origRO = window.ResizeObserver;
  roCallback = null;
  window.ResizeObserver = function (cb) {
    roCallback = cb;
    return { observe: vi.fn(), disconnect: vi.fn() };
  };

  Object.defineProperty(window, 'devicePixelRatio', { value: 2, configurable: true });
});

afterEach(() => {
  HTMLCanvasElement.prototype.getContext = origGetContext;
  window.requestAnimationFrame = origRAF;
  window.cancelAnimationFrame = origCARAF;
  window.ResizeObserver = origRO;
  document.body.innerHTML = '';
  delete window.sendAction;
  roCallback = null;
});

function setup(opts) {
  opts = opts || {};
  if (opts.sendAction) {
    window.sendAction = opts.sendAction;
  }

  document.body.innerHTML = '<ph-navigation-map id="test-map"></ph-navigation-map>';
  const el = document.getElementById('test-map');
  if (!el) throw new Error('element not found');

  const canvas = el.shadowRoot.querySelector('canvas');

  vi.spyOn(el, 'getBoundingClientRect').mockReturnValue(
    { width: 300, height: 300, left: 0, top: 0, right: 300, bottom: 300 }
  );

  vi.spyOn(canvas, 'getBoundingClientRect').mockReturnValue(
    { width: 300, height: 300, left: 0, top: 0, right: 300, bottom: 300 }
  );

  canvas.width = 600;
  canvas.height = 600;

  const tickRaf = () => {
    if (window.requestAnimationFrame.mock.calls.length > 0) {
      const cb = window.requestAnimationFrame.mock.calls[0][0];
      window.requestAnimationFrame.mock.calls.splice(0, 1);
      cb();
    }
  };

  tickRaf();

  return { el, canvas, fakeCtx, tickRaf };
}

function click(el, x, y) {
  el.dispatchEvent(new MouseEvent('mousedown', { clientX: x, clientY: y, bubbles: true }));
  el.dispatchEvent(new MouseEvent('mouseup', { clientX: x, clientY: y, bubbles: true }));
}

function drag(el, fromX, fromY, toX, toY) {
  el.dispatchEvent(new MouseEvent('mousedown', { clientX: fromX, clientY: fromY, bubbles: true }));
  el.dispatchEvent(new MouseEvent('mousemove', { clientX: toX, clientY: toY, bubbles: true }));
  el.dispatchEvent(new MouseEvent('mouseup', { clientX: toX, clientY: toY, bubbles: true }));
}

describe('PhNavigationMap', () => {
  it('is defined and registered as a custom element', () => {
    expect(customElements.get('ph-navigation-map')).toBeDefined();
  });

  it('creates a shadow root with a canvas and overlay', () => {
    const h = setup();
    expect(h.el.shadowRoot).toBeDefined();
    expect(h.el.shadowRoot.querySelector('canvas')).toBeDefined();
    const overlay = h.el.shadowRoot.getElementById('overlay');
    expect(overlay).toBeDefined();
    expect(overlay.tagName).toBe('DIV');
  });

  it('empty state renders grid and ship marker without error', () => {
    const h = setup();
    expect(() => {
      h.el.state = {};
      h.tickRaf();
    }).not.toThrow();
    expect(h.fakeCtx._calls.fillRect.length).toBeGreaterThan(0);
  });

  it('null state renders without error', () => {
    const h = setup();
    expect(() => {
      h.el.state = null;
      h.tickRaf();
    }).not.toThrow();
  });

  it('renders blips at correct positions using world-to-screen transform', () => {
    const h = setup();
    const state = {
      blips: [
        { uuid: 'a', kind: 'planet', name: 'Alpha', world_x: 1000, world_z: 0, stance: 'friendly' },
        { uuid: 'b', kind: 'ship', name: 'Beta', world_x: -1000, world_z: 0, stance: 'hostile' },
      ],
      range: 5000,
      ship_pos: { x: 0, z: 0 },
      ship_heading: 0,
    };
    h.el.state = state;
    h.tickRaf();
    // With ship at origin, heading 0, range 5000, R=300, scale=0.06
    // Blip at (1000, 0): rx=1000, rz=0 → sx=300+1000*0.06=360, sy=300+0=300
    // Blip at (-1000, 0): rx=-1000, rz=0 → sx=300-1000*0.06=240, sy=300
    expect(h.fakeCtx._calls.arc.length).toBeGreaterThanOrEqual(1);
  });

  it('draws the ship marker at its true world position, not the screen centre', () => {
    const h = setup();
    // Canvas 600x600 → cx=cy=300, R=300, range=5000 → scale=0.06.
    // World-anchored (camera on origin): ship at (2000,0) projects to
    // sx = 300 + 2000*0.06 = 420, sy = 300 (unchanged on z=0).
    h.el.state = {
      blips: [],
      range: 5000,
      ship_pos: { x: 2000, z: 0 },
      ship_heading: 0,
    };
    h.tickRaf();
    // #drawShipMarker is the only code path that calls translate().
    const translateCalls = h.fakeCtx.translate.mock.calls;
    expect(translateCalls.length).toBeGreaterThan(0);
    const [sx, sy] = translateCalls[translateCalls.length - 1];
    expect(sx).toBeCloseTo(420, 1);
    expect(sy).toBeCloseTo(300, 1);
  });

  it('ship marker sits at centre only when the ship is at the world origin', () => {
    const h = setup();
    h.el.state = { blips: [], range: 5000, ship_pos: { x: 0, z: 0 }, ship_heading: 0 };
    h.tickRaf();
    const c = h.fakeCtx.translate.mock.calls;
    const [sx, sy] = c[c.length - 1];
    expect(sx).toBeCloseTo(300, 1);
    expect(sy).toBeCloseTo(300, 1);
  });

  it('waypoint marker renders diamond shape when waypoint is set', () => {
    const h = setup();
    const state = {
      blips: [],
      range: 5000,
      ship_pos: { x: 0, z: 0 },
      ship_heading: 0,
      waypoint: { x: 2000, z: 1000 },
    };
    h.el.state = state;
    h.tickRaf();
    expect(h.fakeCtx._calls.moveTo.length).toBeGreaterThan(0);
  });

  it('tap on empty space dispatches set_navigation_waypoint with world coordinates', () => {
    const sendAction = vi.fn();
    const h = setup({ sendAction });
    h.el.state = {
      blips: [],
      range: 5000,
      ship_pos: { x: 0, z: 0 },
      ship_heading: 0,
    };
    h.tickRaf();

    click(h.canvas, 150, 150);
    expect(sendAction).toHaveBeenCalledWith('set_navigation_waypoint', { x: 0, z: 0 });
  });

  it('tap on empty space at known position uses correct world coords', () => {
    const sendAction = vi.fn();
    const h = setup({ sendAction });
    h.el.state = {
      blips: [],
      range: 5000,
      ship_pos: { x: 500, z: 300 },
      ship_heading: 0,
    };
    h.tickRaf();

    // Click at CSS (150, 75) → buffer (300, 150)
    // cx=300, cy=300, scale=0.06 (R=300, range=5000)
    // World-anchored (camera on origin, north-up), zoom=1, pan=(0,0):
    // nx = (300-300)/(0.06*1) = 0, ny = (150-300)/(0.06*1) = -2500
    // wx = 0, wz = -(-2500) = 2500 (independent of ship_pos)
    click(h.canvas, 150, 75);
    expect(sendAction).toHaveBeenCalledWith('set_navigation_waypoint', { x: 0, z: 2500 });
  });

  it('tap on blip dispatches set_navigation_waypoint with source_uuid', () => {
    const sendAction = vi.fn();
    const h = setup({ sendAction });
    h.el.state = {
      blips: [
        { uuid: 'abc', kind: 'planet', name: 'Alpha', world_x: 1000, world_z: 0, stance: 'friendly' },
      ],
      range: 5000,
      ship_pos: { x: 0, z: 0 },
      ship_heading: 0,
    };
    h.tickRaf();

    // Blip at (1000, 0) → buffer: sx = 300+1000*0.06 = 360, sy = 300
    // CSS: x=180, y=150
    click(h.canvas, 180, 150);
    expect(sendAction).toHaveBeenCalledWith('set_navigation_waypoint', {
      x: 1000,
      z: 0,
      source_uuid: 'abc',
    });
  });

  it('tap on blip shows overlay with entity info', () => {
    const sendAction = vi.fn();
    const h = setup({ sendAction });
    const overlay = h.el.shadowRoot.getElementById('overlay');

    h.el.state = {
      blips: [
        { uuid: 'abc', kind: 'station', name: 'Starbase 7', world_x: 1000, world_z: 0, stance: 'friendly' },
      ],
      range: 5000,
      ship_pos: { x: 0, z: 0 },
      ship_heading: 0,
    };
    h.tickRaf();

    expect(overlay.classList.contains('show')).toBe(false);

    click(h.canvas, 180, 150);
    expect(overlay.classList.contains('show')).toBe(true);

    const nameEl = h.el.shadowRoot.getElementById('ov-name');
    expect(nameEl.textContent).toBe('Starbase 7');
  });

  it('tap far from blips hides overlay', () => {
    const sendAction = vi.fn();
    const h = setup({ sendAction });
    const overlay = h.el.shadowRoot.getElementById('overlay');

    h.el.state = {
      blips: [
        { uuid: 'abc', kind: 'planet', name: 'Alpha', world_x: 1000, world_z: 0, stance: 'friendly' },
      ],
      range: 5000,
      ship_pos: { x: 0, z: 0 },
      ship_heading: 0,
    };
    h.tickRaf();

    click(h.canvas, 180, 150);
    expect(overlay.classList.contains('show')).toBe(true);

    click(h.canvas, 0, 0);
    expect(overlay.classList.contains('show')).toBe(false);
  });

  it('ResizeObserver updates canvas size on host element resize', () => {
    const h = setup();
    const canvas = h.canvas;
    expect(canvas.width).toBe(600);
    expect(canvas.height).toBe(600);

    h.el.getBoundingClientRect.mockReturnValue(
      { width: 400, height: 200, left: 0, top: 0, right: 400, bottom: 200 }
    );
    if (roCallback) roCallback();

    expect(canvas.width).toBe(800);
    expect(canvas.height).toBe(400);
  });

  describe('zoom and pan', () => {
    it('drag longer than 5px prevents tap action', () => {
      const sendAction = vi.fn();
      const h = setup({ sendAction });
      h.el.state = {
        blips: [],
        range: 5000,
        ship_pos: { x: 0, z: 0 },
        ship_heading: 0,
      };
      h.tickRaf();

      drag(h.canvas, 100, 100, 200, 100);
      expect(sendAction).not.toHaveBeenCalled();
    });

    it('drag less than 5px still fires tap action', () => {
      const sendAction = vi.fn();
      const h = setup({ sendAction });
      h.el.state = {
        blips: [],
        range: 5000,
        ship_pos: { x: 0, z: 0 },
        ship_heading: 0,
      };
      h.tickRaf();

      drag(h.canvas, 100, 100, 101, 100);
      // mousedown at CSS (100,100) → buf (200,200), <5px movement → tap at (200,200)
      // nx=(200-300)/0.06=-1666.67, ny=(200-300)/0.06=-1666.67
      // heading=0: wx=-1666.67, wz=-(-1666.67)=1666.67
      expect(sendAction).toHaveBeenCalledTimes(1);
      const call = sendAction.mock.calls[0][1];
      expect(call.x).toBeCloseTo(-1666.67, 1);
      expect(call.z).toBeCloseTo(1666.67, 1);
    });

    it('drag pans the map and subsequent tap uses updated world coords', () => {
      const sendAction = vi.fn();
      const h = setup({ sendAction });
      h.el.state = {
        blips: [],
        range: 5000,
        ship_pos: { x: 0, z: 0 },
        ship_heading: 0,
      };
      h.tickRaf();

      // Drag right by 100 CSS pixels (200 buffer pixels)
      drag(h.canvas, 100, 100, 200, 100);
      // No tap expected (drag > 5px)
      sendAction.mockClear();

      // Now tap at the same CSS position where the drag ended
      // panX = 200 (buf: 400-200), panY = 0
      // Tap at CSS (200, 100) → buf (400, 200) → tap uses press pos (400, 200)
      // nx = (400 - 300 - 200) / 0.06 = -100/0.06 = -1666.67
      // ny = (200 - 300 - 0) / 0.06 = -1666.67
      // heading=0: wx = -1666.67, wz = 1666.67
      click(h.canvas, 200, 100);
      expect(sendAction).toHaveBeenCalledTimes(1);
      const call = sendAction.mock.calls[0][1];
      expect(call.x).toBeCloseTo(-1666.67, 1);
      expect(call.z).toBeCloseTo(1666.67, 1);
    });

    it('wheel zoom changes zoom level and affects tap world coords', () => {
      const sendAction = vi.fn();
      const h = setup({ sendAction });
      h.el.state = {
        blips: [],
        range: 5000,
        ship_pos: { x: 0, z: 0 },
        ship_heading: 0,
      };
      h.tickRaf();

      // Baseline: tap top-left to get world coords at zoom=1
      click(h.canvas, 0, 0);
      const baselineCall = sendAction.mock.calls[0][1];

      sendAction.mockClear();

      // Dispatch wheel event to zoom in
      h.canvas.dispatchEvent(new WheelEvent('wheel', {
        deltaY: -120,
        clientX: 150,
        clientY: 150,
        bubbles: true,
        cancelable: true,
      }));

      // Now tap top-left again — it should map to a different world position (closer to ship)
      click(h.canvas, 0, 0);
      const zoomedCall = sendAction.mock.calls[0][1];

      // At zoom=1, world coords at (0,0) CSS center:
      //   nx=(0-300)/0.06 = -5000, ny=(0-300)/0.06 = -5000, wx=-5000, wz=-5000
      // At zoom=1.13, same CSS (0,0):
      //   nx=(0-300)/(0.06*1.13) = -300/0.0678 = -4424.78, wx=-4424.78
      expect(Math.abs(zoomedCall.x)).toBeLessThan(Math.abs(baselineCall.x));
      expect(Math.abs(zoomedCall.z)).toBeLessThan(Math.abs(baselineCall.z));
    });

    it('zoom and pan survive state updates', () => {
      const sendAction = vi.fn();
      const h = setup({ sendAction });
      h.el.state = {
        blips: [],
        range: 5000,
        ship_pos: { x: 0, z: 0 },
        ship_heading: 0,
      };
      h.tickRaf();

      // Zoom in (wheel at center → zoom=1.13)
      h.canvas.dispatchEvent(new WheelEvent('wheel', {
        deltaY: -120,
        clientX: 150,
        clientY: 150,
        bubbles: true,
        cancelable: true,
      }));
      // Drag right by 50 CSS pixels (100 buffer px) → panX=100
      drag(h.canvas, 150, 100, 200, 100);

      sendAction.mockClear();

      // Update state (simulating new server data)
      h.el.state = {
        blips: [],
        range: 5000,
        ship_pos: { x: 100, z: 100 },
        ship_heading: 0,
      };
      h.tickRaf();

      // Tap at center CSS (150,150) → buf (300,300)
      // World-anchored (camera on origin), zoom=1.13, panX=100, panY=0:
      //   nx = (300-300-100)/(0.06*1.13) = -100/0.0678 = -1474.93
      //   ny = (300-300-0)/(0.06*1.13) = 0
      //   wx = -1474.93, wz = 0 (independent of ship_pos)
      click(h.canvas, 150, 150);
      expect(sendAction).toHaveBeenCalledTimes(1);
      const call = sendAction.mock.calls[0][1];
      expect(call.x).toBeCloseTo(-1474.93, 1);
      expect(call.z).toBe(0);
    });

    it('mousemove without prior mousedown does not affect pan', () => {
      const sendAction = vi.fn();
      const h = setup({ sendAction });
      h.el.state = {
        blips: [],
        range: 5000,
        ship_pos: { x: 0, z: 0 },
        ship_heading: 0,
      };
      h.tickRaf();

      // Dispatch mousemove without preceding mousedown (should be no-op)
      h.canvas.dispatchEvent(new MouseEvent('mousemove', { clientX: 200, clientY: 200, bubbles: true }));

      // Tap should still work with default pan
      click(h.canvas, 150, 150);
      expect(sendAction).toHaveBeenCalledWith('set_navigation_waypoint', { x: 0, z: 0 });
    });

    it('wheel zoom clamps to min and max', () => {
      const sendAction = vi.fn();
      const h = setup({ sendAction });
      h.el.state = {
        blips: [],
        range: 5000,
        ship_pos: { x: 0, z: 0 },
        ship_heading: 0,
      };
      h.tickRaf();

      // Zoom out many times (deltaY > 0 = zoom out)
      for (let i = 0; i < 20; i++) {
        h.canvas.dispatchEvent(new WheelEvent('wheel', {
          deltaY: 120,
          clientX: 150,
          clientY: 150,
          bubbles: true,
          cancelable: true,
        }));
      }

      // Compute expected zoom after 20 zoom-outs (factor 0.885 each)
      // 1 * 0.885^20 ≈ 0.089, clamped to 0.25
      // Tap at top-left after extreme zoom out
      sendAction.mockClear();
      click(h.canvas, 0, 0);
      const call = sendAction.mock.calls[0][1];
      // At zoom=0.25 (clamped), top-left maps to:
      // nx = (0-300)/(0.06*0.25) = -300/0.015 = -20000
      expect(call.x).toBeCloseTo(-20000, 0);
    });
  });
});
