// @vitest-environment jsdom
import { t } from '../../gui/strings.js';
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
    expect(el.shadowRoot.getElementById('torpedo-badges')).toBeDefined();
  });

  // ── Torpedo-armed marker (issue #957) ──────────────────────────────────────

  it('draws a torpedo-armed badge beside a contact the state marked, before it fires', () => {
    const { el } = setup();
    // radar_x=0, radar_y=0.5 → bx = 50 + 0*46 = 50, by = 50 - 0.5*46 = 27
    el.state = {
      blips: [
        { uuid: 'torp-boat', radar_x: 0, radar_y: 0.5, torpedo_badge: t('console.radar.torpedo_armed') },
      ],
    };
    const labels = el.shadowRoot.getElementById('torpedo-badges').querySelectorAll('text');
    expect(labels.length).toBe(1);
    expect(labels[0].textContent).toBe(t('console.radar.torpedo_armed'));
    expect(labels[0].getAttribute('data-uuid')).toBe('torp-boat');
    expect(labels[0].getAttribute('x')).toBe('53.0');
    expect(labels[0].getAttribute('y')).toBe('24.0');
  });

  it('badges only the contacts the state flagged', () => {
    const { el } = setup();
    el.state = {
      blips: [
        { uuid: 'phaser-boat', radar_x: 0.2, radar_y: 0.2 },
        { uuid: 'torp-boat', radar_x: 0, radar_y: 0.5, torpedo_badge: 'TORP' },
      ],
    };
    const labels = el.shadowRoot.getElementById('torpedo-badges').querySelectorAll('text');
    expect(labels.length).toBe(1);
    expect(labels[0].getAttribute('data-uuid')).toBe('torp-boat');
  });

  it('clears badges when the contact is gone from a later state', () => {
    const { el } = setup();
    el.state = { blips: [{ uuid: 'torp-boat', radar_x: 0, radar_y: 0.5, torpedo_badge: 'TORP' }] };
    expect(el.shadowRoot.getElementById('torpedo-badges').children.length).toBe(1);
    el.state = { blips: [] };
    expect(el.shadowRoot.getElementById('torpedo-badges').children.length).toBe(0);
  });

  it('passes base state through to inner ph-radar', () => {
    const { el, innerRadar } = setup();
    el.state = {
      blips: [{ uuid: 'a', radar_x: 0, radar_y: 0.5 }],
      ship_heading: 90,
      config: { max_range: 5000 },
    };
    expect(innerRadar.state).toEqual({
      blips: [{ uuid: 'a', radar_x: 0, radar_y: 0.5 }],
      ship_heading: 90,
      config: { max_range: 5000 },
      target_uuid: null,
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
    // radar_x=0, radar_y=0.5 → bx = 50 + 0*46 = 50, by = 50 - 0.5*46 = 27
    el.state = {
      blips: [{ uuid: 'abc', radar_x: 0, radar_y: 0.5 }],
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
      blips: [{ uuid: 'abc', radar_x: 0, radar_y: 0.5 }],
      selected_target_uuid: null,
    };
    const g = el.shadowRoot.getElementById('selected-highlight');
    expect(g.childNodes.length).toBe(0);
  });

  it('blip click on inner radar dispatches sendAction via wrapper', () => {
    const sendAction = vi.fn();
    const { el, canvas, tickRaf } = setup({ sendAction });
    // radar_x=0, radar_y=0.5 → bx = 300 + 0*300 = 300, by = 300 - 0.5*300 = 150
    // CSS: x=150, y=75
    el.state = {
      blips: [{ uuid: 'abc', radar_x: 0, radar_y: 0.5, color: '#ff0000' }],
    };
    tickRaf();
    canvas.dispatchEvent(new MouseEvent('click', { clientX: 150, clientY: 75 }));
    expect(sendAction).toHaveBeenCalledWith('set_target', { uuid: 'abc' });
  });
});
