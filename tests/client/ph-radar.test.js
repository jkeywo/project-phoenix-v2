// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import '../../gui/components/ph-radar.js';

function makeFakeCtx() {
  const calls = { fillRect: [], arc: [], fill: [], drawImage: [], fillText: [] };
  const ctx = {
    _calls: calls,
    fillStyle: '',
    fillRect: (...a) => calls.fillRect.push(a),
    beginPath: vi.fn(),
    arc: (...a) => calls.arc.push(a),
    fill: () => calls.fill.push(true),
    drawImage: (...a) => calls.drawImage.push(a),
    fillText: (...a) => calls.fillText.push(a),
    font: '',
  };
  return ctx;
}

// ── Shared mocks (applied before each test) ────────────────────────────

let fakeCtx;
let origGetContext;
let origRAF;
let origCARAF;
let origRO;
let origImage;
let roCallback;

beforeEach(() => {
  fakeCtx = makeFakeCtx();

  // Canvas getContext — must use function(){} not arrow to allow new
  origGetContext = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = function () { return fakeCtx; };

  // RAF: store callback so tests can tick manually
  let rafCb = null;
  origRAF = window.requestAnimationFrame;
  window.requestAnimationFrame = vi.fn((cb) => { rafCb = cb; return 1; });
  origCARAF = window.cancelAnimationFrame;
  window.cancelAnimationFrame = vi.fn();

  // ResizeObserver — must use function(){} not arrow to allow new
  origRO = window.ResizeObserver;
  roCallback = null;
  window.ResizeObserver = function (cb) {
    roCallback = cb;
    return { observe: vi.fn(), disconnect: vi.fn() };
  };

  // Image mock: always "loaded"
  origImage = window.Image;
  window.Image = class {
    constructor() { this.naturalWidth = 64; this.naturalHeight = 64; this.complete = true; }
  };

  // devicePixelRatio
  Object.defineProperty(window, 'devicePixelRatio', { value: 2, configurable: true });
});

afterEach(() => {
  HTMLCanvasElement.prototype.getContext = origGetContext;
  window.requestAnimationFrame = origRAF;
  window.cancelAnimationFrame = origCARAF;
  window.ResizeObserver = origRO;
  window.Image = origImage;
  document.body.innerHTML = '';
  delete window.sendAction;
  roCallback = null;
});

// ── Helpers ──────────────────────────────────────────────────────────

function setup(opts) {
  opts = opts || {};
  // Set sendAction BEFORE element creation so connectedCallback picks it up
  if (opts.sendAction) {
    window.sendAction = opts.sendAction;
  }

  document.body.innerHTML = '<ph-radar id="test-radar"></ph-radar>';
  const el = document.getElementById('test-radar');
  if (!el) throw new Error('element not found — constructor may have thrown');

  const canvas = el.shadowRoot.querySelector('canvas');

  // Mock getBoundingClientRect on element for ResizeObserver sizing
  vi.spyOn(el, 'getBoundingClientRect').mockReturnValue(
    { width: 300, height: 300, left: 0, top: 0, right: 300, bottom: 300 }
  );

  // Also mock on the canvas for click coordinate mapping
  vi.spyOn(canvas, 'getBoundingClientRect').mockReturnValue(
    { width: 300, height: 300, left: 0, top: 0, right: 300, bottom: 300 }
  );

  // Manually size canvas to 600×600 (300 * 2 DPR)
  canvas.width = 600;
  canvas.height = 600;

  const tickRaf = () => {
    if (window.requestAnimationFrame.mock.calls.length > 0) {
      const cb = window.requestAnimationFrame.mock.calls[0][0];
      window.requestAnimationFrame.mock.calls.splice(0, 1);
      cb();
    }
  };

  // Drain initial setup rAF to get past the initial render attempt
  tickRaf();

  const cleanup = () => {};

  return { el, canvas, fakeCtx, tickRaf, cleanup };
}

// ── Tests ────────────────────────────────────────────────────────────

describe('PhRadar', () => {
  // ── Registration & structure ──────────────────────────────────────

  it('is defined and registered as a custom element', () => {
    expect(customElements.get('ph-radar')).toBeDefined();
  });

  it('creates a shadow root with a canvas', () => {
    const h = setup();
    expect(h.el.shadowRoot).toBeDefined();
    expect(h.el.shadowRoot.querySelector('canvas')).toBeDefined();
  });

  // ── Render on state set ──────────────────────────────────────────

  it('state setter triggers render (canvas draw calls)', () => {
    const h = setup();
    const before = h.fakeCtx._calls.fillRect.length;
    h.el.state = { blips: [{ uuid: 'a', radar_x: 0, radar_y: 0.5 }] };
    h.tickRaf();
    expect(h.fakeCtx._calls.fillRect.length).toBeGreaterThan(before);
  });

  // ── Empty state ──────────────────────────────────────────────────

  it('empty state renders without error', () => {
    const h = setup();
    expect(() => {
      h.el.state = {};
      h.tickRaf();
    }).not.toThrow();
  });

  it('null state renders without error', () => {
    const h = setup();
    expect(() => {
      h.el.state = null;
      h.tickRaf();
    }).not.toThrow();
  });

  it('state with undefined blips renders without error', () => {
    const h = setup();
    expect(() => {
      h.el.state = { blips: undefined, range: 500 };
      h.tickRaf();
    }).not.toThrow();
  });

  // ── Hit testing / target selection ───────────────────────────────

  it('click on blip calls sendAction with set_target and uuid', () => {
    const sendAction = vi.fn();
    const h = setup({ sendAction });
    // radar_x=0, radar_y=0.5 → bx = 300 + 0*300 = 300, by = 300 - 0.5*300 = 150
    // CSS: x=150, y=75
    h.el.state = {
      blips: [{ uuid: 'abc', radar_x: 0, radar_y: 0.5, color: '#ff0000' }],
    };
    h.tickRaf();

    h.canvas.dispatchEvent(new MouseEvent('click', { clientX: 150, clientY: 75 }));
    expect(sendAction).toHaveBeenCalledWith('set_target', { uuid: 'abc' });
  });

  it('click on blip at starboard fires correctly', () => {
    const sendAction = vi.fn();
    const h = setup({ sendAction });
    // radar_x=0.5, radar_y=0 → bx = 300 + 0.5*300 = 450, by = 300 - 0*300 = 300
    // CSS: x=225, y=150
    h.el.state = {
      blips: [{ uuid: 'def', radar_x: 0.5, radar_y: 0, color: '#00ff00' }],
    };
    h.tickRaf();

    h.canvas.dispatchEvent(new MouseEvent('click', { clientX: 225, clientY: 150 }));
    expect(sendAction).toHaveBeenCalledWith('set_target', { uuid: 'def' });
  });

  it('click far from blips does nothing', () => {
    const sendAction = vi.fn();
    const h = setup({ sendAction });
    h.el.state = {
      blips: [{ uuid: 'abc', radar_x: 0, radar_y: 0.5, color: '#ff0000' }],
    };
    h.tickRaf();

    // Click at top-left corner — far from blip at (300,150) buffer → (150,75) CSS
    h.canvas.dispatchEvent(new MouseEvent('click', { clientX: 0, clientY: 0 }));
    expect(sendAction).not.toHaveBeenCalled();
  });

  it('no sendAction set does not throw on blip click', () => {
    const h = setup(); // no sendAction
    h.el.state = {
      blips: [{ uuid: 'abc', radar_x: 0, radar_y: 0.5, color: '#ff0000' }],
    };
    h.tickRaf();

    expect(() => {
      h.canvas.dispatchEvent(new MouseEvent('click', { clientX: 150, clientY: 75 }));
    }).not.toThrow();
  });

  it('sendAction from window.sendAction is picked up by connectedCallback', () => {
    const sendAction = vi.fn();
    const h = setup({ sendAction });
    expect(h.el.sendAction).toBe(sendAction);
  });

  // ── Pre-projected blip positioning ──────────────────────────────

  it('radar_x/radar_y positions blip for hit-testing', () => {
    const sendAction = vi.fn();
    const h = setup({ sendAction });
    // radar_x=-0.5, radar_y=0 → bx = 300 + (-0.5)*300 = 150, by = 300 - 0*300 = 300
    // CSS: x=75, y=150
    h.el.state = {
      blips: [{ uuid: 'abc', radar_x: -0.5, radar_y: 0, color: '#ff0000' }],
    };
    h.tickRaf();

    h.canvas.dispatchEvent(new MouseEvent('click', { clientX: 75, clientY: 150 }));
    expect(sendAction).toHaveBeenCalledWith('set_target', { uuid: 'abc' });
  });

  // ── ResizeObserver ───────────────────────────────────────────────

  it('ResizeObserver updates canvas size on host element resize', () => {
    const h = setup();
    const canvas = h.canvas;
    expect(canvas.width).toBe(600);
    expect(canvas.height).toBe(600);

    // Change element size and trigger ResizeObserver callback
    h.el.getBoundingClientRect.mockReturnValue(
      { width: 400, height: 200, left: 0, top: 0, right: 400, bottom: 200 }
    );
    if (roCallback) roCallback();
    // The resize observer callback sets needsRender, but the canvas size
    // update happens internally. Since we mocked the rAF, we need to
    // tick manually to cause the render which calls updateSize... Actually,
    // updateSize is called directly by the ResizeObserver callback, not
    // via rAF. So the canvas should already be resized.

    expect(canvas.width).toBe(800); // 400 * 2
    expect(canvas.height).toBe(400); // 200 * 2
  });

  // ── disconnectedCallback ─────────────────────────────────────────

  it('disconnectedCallback cancels the rAF loop', () => {
    const h = setup();
    expect(window.cancelAnimationFrame).not.toHaveBeenCalled();
    h.el.remove();
    expect(window.cancelAnimationFrame).toHaveBeenCalled();
  });

  it('disconnectedCallback cleans up without error', () => {
    const h = setup();
    expect(() => h.el.remove()).not.toThrow();
    // Removing again should also not throw
    expect(() => {
      const el2 = document.createElement('ph-radar');
      el2.remove();
    }).not.toThrow();
  });

  // ── Icon loading ─────────────────────────────────────────────────

  it('blip with icon uses drawImage', () => {
    const h = setup();
    h.el.state = {
      blips: [{
        uuid: 'icon-blip',
        radar_x: 0,
        radar_y: 0.5,
        icon: 'warbird',
      }],
    };
    // The Image mock has complete=true, naturalWidth>0, so icon is "loaded"
    h.tickRaf();
    expect(h.fakeCtx._calls.drawImage.length).toBeGreaterThan(0);
  });

  it('blip without icon uses arc + fill', () => {
    const h = setup();
    h.el.state = {
      blips: [{
        uuid: 'circle-blip',
        radar_x: 0,
        radar_y: 0.5,
        color: '#ff3344',
      }],
    };
    h.tickRaf();
    // Should have at least 2 arcs: disc background + blip
    expect(h.fakeCtx._calls.arc.length).toBeGreaterThanOrEqual(2);
  });
});
