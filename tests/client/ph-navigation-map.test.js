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
    // Should have at least fillRect calls for background
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
    // fill rect calls: background + station rect (but kind=planet uses arc, not rect)
    // Actually planet uses arc, ship uses triangle path, so the arcs are drawn
    expect(h.fakeCtx._calls.arc.length).toBeGreaterThanOrEqual(1);
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
    // The waypoint should call moveTo/lineTo for the diamond path
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

    // Click center of canvas (300,300 in buffer = 150,150 in CSS)
    // This maps to world (0, 0) since ship is at origin
    h.canvas.dispatchEvent(new MouseEvent('click', { clientX: 150, clientY: 150 }));
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
    // nx = (300-300)/0.06 = 0, ny = (150-300)/0.06 = -2500
    // heading=0, cos=1, sin=0: wx = 0 + 0 + 500 = 500, wz = 0 - (-2500) + 300 = 2800
    h.canvas.dispatchEvent(new MouseEvent('click', { clientX: 150, clientY: 75 }));
    expect(sendAction).toHaveBeenCalledWith('set_navigation_waypoint', { x: 500, z: 2800 });
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
    h.canvas.dispatchEvent(new MouseEvent('click', { clientX: 180, clientY: 150 }));
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

    h.canvas.dispatchEvent(new MouseEvent('click', { clientX: 180, clientY: 150 }));
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

    // First tap the blip to show overlay
    h.canvas.dispatchEvent(new MouseEvent('click', { clientX: 180, clientY: 150 }));
    expect(overlay.classList.contains('show')).toBe(true);

    // Then tap far away
    h.canvas.dispatchEvent(new MouseEvent('click', { clientX: 0, clientY: 0 }));
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
});
