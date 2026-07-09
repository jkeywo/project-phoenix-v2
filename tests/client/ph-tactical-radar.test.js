// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import '../../gui/components/ph-tactical-radar.js';

function makeFakeCtx() {
  const calls = { fillRect: [], arc: [], fill: [], drawImage: [], fillText: [] };
  return {
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
}

let fakeCtx;
let origGetContext;
let origRAF;
let origCARAF;
let origRO;
let origImage;
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
  origImage = window.Image;
  window.Image = class {
    constructor() { this.naturalWidth = 64; this.naturalHeight = 64; this.complete = true; }
  };
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

function setup(opts) {
  opts = opts || {};
  if (opts.sendAction) {
    window.sendAction = opts.sendAction;
  }
  document.body.innerHTML = '<ph-tactical-radar id="test-el"></ph-tactical-radar>';
  const el = document.getElementById('test-el');
  if (!el) throw new Error('element not found');
  const innerRadar = el.shadowRoot.getElementById('inner-radar');
  const canvas = innerRadar.shadowRoot.querySelector('canvas');
  vi.spyOn(el, 'getBoundingClientRect').mockReturnValue(
    { width: 300, height: 300, left: 0, top: 0, right: 300, bottom: 300 }
  );
  vi.spyOn(innerRadar, 'getBoundingClientRect').mockReturnValue(
    { width: 300, height: 300, left: 0, top: 0, right: 300, bottom: 300 }
  );
  if (canvas) {
    vi.spyOn(canvas, 'getBoundingClientRect').mockReturnValue(
      { width: 300, height: 300, left: 0, top: 0, right: 300, bottom: 300 }
    );
    canvas.width = 600;
    canvas.height = 600;
  }
  const tickRaf = () => {
    if (window.requestAnimationFrame.mock.calls.length > 0) {
      const cb = window.requestAnimationFrame.mock.calls[0][0];
      window.requestAnimationFrame.mock.calls.splice(0, 1);
      cb();
    }
  };
  tickRaf();
  return { el, innerRadar, canvas, fakeCtx, tickRaf };
}

describe('PhTacticalRadar', () => {
  it('is defined and registered as a custom element', () => {
    expect(customElements.get('ph-tactical-radar')).toBeDefined();
  });

  it('creates a shadow root with inner ph-radar and SVG overlay groups', () => {
    const { el } = setup();
    expect(el.shadowRoot).toBeDefined();
    expect(el.shadowRoot.getElementById('inner-radar')).toBeDefined();
    expect(el.shadowRoot.getElementById('phaser-arcs')).toBeDefined();
    expect(el.shadowRoot.getElementById('torpedo-arcs')).toBeDefined();
    expect(el.shadowRoot.getElementById('selected-highlight')).toBeDefined();
  });

  it('passes base state through to inner ph-radar', () => {
    const { el, innerRadar } = setup();
    el.state = {
      blips: [{ uuid: 'a', bearing_deg: 0, range: 500 }],
      range: 1000,
      ship_heading: 90,
      config: { max_range: 5000 },
    };
    expect(innerRadar.state).toEqual({
      blips: [{ uuid: 'a', bearing_deg: 0, range: 500 }],
      range: 1000,
      ship_heading: 90,
      config: { max_range: 5000 },
    });
  });

  it('phaser arcs render SVG wedge paths in phaser-arcs group', () => {
    const { el } = setup();
    el.state = {
      phaser_arcs: [{ facing_deg: 0, arc_deg: 270, color: '#4ec870' }],
    };
    const g = el.shadowRoot.getElementById('phaser-arcs');
    const paths = g.querySelectorAll('path');
    expect(paths.length).toBe(1);
    expect(paths[0].getAttribute('fill')).toBe('#4ec870');
    expect(paths[0].getAttribute('d')).toBeTruthy();
    expect(paths[0].getAttribute('fill-opacity')).toBe('0.3');
  });

  it('torpedo arcs render with default color when no color specified', () => {
    const { el } = setup();
    el.state = {
      torpedo_arcs: [{ facing_deg: 180, arc_deg: 90 }],
    };
    const g = el.shadowRoot.getElementById('torpedo-arcs');
    const paths = g.querySelectorAll('path');
    expect(paths.length).toBe(1);
    expect(paths[0].getAttribute('d')).toBeTruthy();
  });

  it('selected target highlight renders circle around blip position', () => {
    const { el } = setup();
    el.state = {
      blips: [{ uuid: 'abc', bearing_deg: 0, range: 500 }],
      range: 1000,
      ship_heading: 0,
      selected_target_uuid: 'abc',
    };
    const g = el.shadowRoot.getElementById('selected-highlight');
    const circles = g.querySelectorAll('circle');
    expect(circles.length).toBe(1);
    expect(circles[0].getAttribute('stroke')).toBe('#6cb6d0');
    expect(circles[0].getAttribute('fill')).toBe('none');
    expect(circles[0].getAttribute('cx')).toBe('50.0');
    expect(circles[0].getAttribute('cy')).toBe('27.0');
  });

  it('no highlight rendered when selected_target_uuid is null', () => {
    const { el } = setup();
    el.state = {
      blips: [{ uuid: 'abc', bearing_deg: 0, range: 500 }],
      range: 1000,
      selected_target_uuid: null,
    };
    const g = el.shadowRoot.getElementById('selected-highlight');
    expect(g.childNodes.length).toBe(0);
  });

  it('blip click on inner radar dispatches sendAction via wrapper', () => {
    const sendAction = vi.fn();
    const { el, canvas, tickRaf } = setup({ sendAction });
    el.state = {
      blips: [{ uuid: 'abc', bearing_deg: 0, range: 500, color: '#ff0000' }],
      range: 1000,
      ship_heading: 0,
    };
    tickRaf();
    // blip at bearing 0 (north), range 500 in 1000 max, canvas 600x600
    // center (300,300), rangeFrac=0.5, dist=150
    // angle = 0 - 0 - π/2 = -π/2
    // bx = 300 + 150*cos(-π/2) = 300
    // by = 300 + 150*sin(-π/2) = 150
    // CSS coords: /2 => (150, 75)
    canvas.dispatchEvent(new MouseEvent('click', { clientX: 150, clientY: 75 }));
    expect(sendAction).toHaveBeenCalledWith('set_target', { uuid: 'abc' });
  });
});
