// @vitest-environment jsdom
import { t } from '../../gui/strings.js';
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import '../../gui/components/ph-radar.js';
import { makeRadarCtx } from './radar-canvas-stub.js';

function makeFakeCtx() {
  return makeRadarCtx();
}

// ── Shared mocks (applied before each test) ────────────────────────────

let fakeCtx;
let origGetContext;
let origRAF;
let origCARAF;
let origRO;
let origImage;
let roCallback;
let imageSources;

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
  imageSources = [];
  window.Image = class {
    constructor() { this.naturalWidth = 64; this.naturalHeight = 64; this.complete = true; }
    set src(value) { this._src = value; imageSources.push(value); }
    get src() { return this._src; }
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

  // The scope is `cssSize` CSS pixels square with a backing store `dpr` times
  // that — the real arrangement, and the one the pixel-ratio tests vary.
  const cssSize = opts.cssSize || 300;
  const dpr = opts.dpr || 2;
  const rect = { width: cssSize, height: cssSize, left: 0, top: 0, right: cssSize, bottom: cssSize };

  // Mock getBoundingClientRect on element for ResizeObserver sizing
  vi.spyOn(el, 'getBoundingClientRect').mockReturnValue(rect);

  // Also mock on the canvas for click coordinate mapping
  vi.spyOn(canvas, 'getBoundingClientRect').mockReturnValue(rect);

  canvas.width = cssSize * dpr;
  canvas.height = cssSize * dpr;

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

  return { el, canvas, fakeCtx, tickRaf, cleanup, cssSize, dpr };
}

/** Tap the scope at a point given in CSS pixels from its top-left corner. */
function tapCss(h, cssX, cssY) {
  h.canvas.dispatchEvent(new MouseEvent('click', {
    clientX: cssX, clientY: cssY, bubbles: true,
  }));
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

  it('layers the static surround below a rotating radar screen and blips', () => {
    const h = setup();
    expect(imageSources).toEqual(expect.arrayContaining([
      '../../assets/helm_console/radar-bg.png',
      '../../assets/helm_console/radar-surround.png',
    ]));

    h.el.state = {
      ship_heading: 90,
      blips: [{ uuid: 'contact', radar_x: 0, radar_y: 0.5, icon: 'warbird' }],
    };
    h.tickRaf();

    const images = h.fakeCtx._calls.drawImage.map(args => args[0]?.src);
    const surround = images.indexOf('../../assets/helm_console/radar-surround.png');
    const background = images.indexOf('../../assets/helm_console/radar-bg.png');
    const blip = images.lastIndexOf('../../assets/radar_icons/Icon-Warbird.png');
    expect(surround).toBeGreaterThanOrEqual(0);
    expect(background).toBeGreaterThan(surround);
    expect(blip).toBeGreaterThan(background);
    expect(h.fakeCtx._calls.rotate).toContainEqual([-Math.PI / 2]);
  });

  it('renders artwork even when there are no blips', () => {
    const h = setup();
    h.el.state = { blips: [] };
    h.tickRaf();
    const images = h.fakeCtx._calls.drawImage.map(args => args[0]?.src);
    expect(images).toContain('../../assets/helm_console/radar-surround.png');
    expect(images).toContain('../../assets/helm_console/radar-bg.png');
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

  // ── Device pixel ratio (PRD #1023 defect) ────────────────────────
  //
  // The backing store is sized rect × devicePixelRatio, so a bare number in a
  // draw call is a DEVICE pixel. Every fixed size the scope drew was authored
  // as a bare number, which made labels and tap targets shrink by 1/dpr — on a
  // 3× phone, 11px labels rendered at 3.7 CSS px and the 14px tap radius at
  // 4.7. These assert the sizes in the units the player actually experiences.

  it('renders blip labels at the same CSS size whatever the pixel ratio', () => {
    const labelled = { blips: [{ uuid: 'a', radar_x: 0, radar_y: 0, label: 'HARROW' }] };

    const one = setup({ dpr: 1 });
    one.el.state = labelled;
    one.tickRaf();
    expect(one.fakeCtx.font).toBe('11px "JetBrains Mono", monospace');

    const three = setup({ dpr: 3 });
    three.el.state = labelled;
    three.tickRaf();
    // 33 buffer px ÷ 3 = the same 11 CSS px the 1× scope draws.
    expect(three.fakeCtx.font).toBe('33px "JetBrains Mono", monospace');
  });

  it('keeps the blip tap target at its CSS size on a high-DPI screen', () => {
    const hits = [];
    const h = setup({ dpr: 3, cssSize: 300, sendAction: (a, p) => hits.push([a, p]) });
    h.el.state = { blips: [{ uuid: 'contact', radar_x: 0, radar_y: 0 }] };
    h.tickRaf();

    // The blip sits at the centre of the scope. The hit radius is 14 CSS px,
    // so a finger 13 CSS px off-centre selects it. Before the fix the radius
    // was 14 BUFFER px — 4.7 CSS px — and this tap missed by a mile.
    tapCss(h, 150 + 13, 150);
    expect(hits).toEqual([['set_target', { uuid: 'contact' }]]);
  });

  it('still rejects a tap outside the blip tap target', () => {
    const hits = [];
    const h = setup({ dpr: 3, cssSize: 300, sendAction: (a, p) => hits.push([a, p]) });
    h.el.state = { blips: [{ uuid: 'contact', radar_x: 0, radar_y: 0 }] };
    h.tickRaf();

    // 40 CSS px away — comfortably outside the 14 CSS px radius. The floor
    // grows with the ratio; it does not become unbounded.
    tapCss(h, 150 + 40, 150);
    expect(hits).toEqual([]);
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
    // The supplied background artwork replaces the fallback disc; the blip
    // itself still draws as a canvas arc.
    expect(h.fakeCtx._calls.arc.length).toBeGreaterThanOrEqual(1);
  });
});
