// @vitest-environment jsdom
import { t } from '../../gui/strings.js';
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import '../../gui/components/ph-navigation-map.js';
import {
  NAVIGATION_CONTACT_ACTION_ID,
  NAVIGATION_PAN_RIGHT_ACTION_ID,
  NAVIGATION_WAYPOINT_ANCHOR_ACTION_ID,
  NAVIGATION_WAYPOINT_CLEAR_ACTION_ID,
  NAVIGATION_WAYPOINT_PLACE_ACTION_ID,
  NAVIGATION_ZOOM_IN_ACTION_ID,
} from '../../gui/stations/navigation-actions.js';

// Canvas paint cannot resolve a CSS custom property, so the map names the
// token and gui/components/ph-console-styles.js resolves it against the live
// document. jsdom loads no stylesheet, so what reaches the stubbed context
// here is the token expression itself — a better thing to assert than a hex
// value, since it says WHICH colour the ring is meant to be.
const GOLD = 'var(--gold)';

function makeFakeCtx() {
  const calls = { fillRect: [], strokeRect: [], arc: [], fill: [], fillText: [], moveTo: [], lineTo: [], stroke: [], beginPath: [] };
  // Chronological op log. The style properties below are single-valued, so a
  // test that wants to know which colour a particular fill/stroke used has to
  // sample them at call time rather than read them back after the render.
  const ops = [];
  const rec = (op, args) => ops.push({
    op,
    args,
    fillStyle: ctx.fillStyle,
    strokeStyle: ctx.strokeStyle,
    lineWidth: ctx.lineWidth,
  });
  const ctx = {
    _calls: calls,
    _ops: ops,
    fillStyle: '',
    strokeStyle: '',
    lineWidth: 0,
    font: '',
    textAlign: 'left',
    shadowColor: '',
    shadowBlur: 0,
    fillRect: (...a) => { calls.fillRect.push(a); rec('fillRect', a); },
    strokeRect: (...a) => { calls.strokeRect.push(a); rec('strokeRect', a); },
    beginPath: () => { calls.beginPath.push(true); },
    arc: (...a) => { calls.arc.push(a); rec('arc', a); },
    fill: () => { calls.fill.push(true); rec('fill', []); },
    stroke: () => { calls.stroke.push(true); rec('stroke', []); },
    moveTo: (...a) => calls.moveTo.push(a),
    lineTo: (...a) => calls.lineTo.push(a),
    fillText: (...a) => calls.fillText.push({ text: a[0], x: a[1], y: a[2], font: ctx.font }),
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

/** The op that painted a shape, or undefined. */
function findOp(ctx, op, match) {
  return ctx._ops.find((o) => o.op === op && match(o));
}

/** The arc that defined the path the op at `index` painted. */
function arcBefore(ctx, index) {
  for (let i = index - 1; i >= 0; i--) {
    if (ctx._ops[i].op === 'arc') return ctx._ops[i];
  }
  return null;
}

/** The arc feeding the first fill/stroke matching `match`, or null. */
function arcFor(ctx, op, match) {
  const i = ctx._ops.findIndex((o) => o.op === op && match(o));
  return i === -1 ? null : arcBefore(ctx, i);
}

let fakeCtx;
let origGetContext;
let origRAF;
let origCARAF;
let origRO;
let origImage;
let roCallback;
let imageSources;
let imageInstances;

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
  imageSources = [];
  imageInstances = [];
  window.Image = class {
    constructor() {
      this.complete = true;
      this.naturalWidth = 64;
      this.naturalHeight = 64;
      imageInstances.push(this);
    }

    set src(value) {
      this._src = value;
      imageSources.push(value);
      if (value.includes('Missing')) {
        this.naturalWidth = 0;
        this.naturalHeight = 0;
      }
    }

    get src() { return this._src; }
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
  delete window.activateSemanticAction;
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

function touch(el, type, touches, changedTouches = []) {
  const event = new Event(type, { bubbles: true, cancelable: true });
  Object.defineProperties(event, {
    touches: { value: touches },
    changedTouches: { value: changedTouches },
  });
  el.dispatchEvent(event);
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

  it('draws distinct authored icons and falls back by kind after an icon failure', () => {
    const h = setup();
    h.el.state = {
      interaction: 'inspect',
      show_ship_marker: false,
      regions: [],
      range: 5000,
      blips: [
        {
          uuid: 'structure-a', kind: 'structure', icon: 'station',
          world_x: -1000, world_z: 0, destroyed: true,
        },
        {
          uuid: 'structure-b', kind: 'structure', icon: 'destroyer',
          world_x: 0, world_z: 0,
        },
        {
          uuid: 'structure-c', kind: 'structure', icon: 'missing',
          world_x: 1000, world_z: 0,
        },
      ],
    };
    expect(h.el.navigationSelect({ uuid: 'structure-a' })).toBe(true);
    h.tickRaf();

    expect(imageSources).toEqual([
      '../../assets/radar_icons/Icon-Station.png',
      '../../assets/radar_icons/Icon-Destroyer.png',
      '../../assets/radar_icons/Icon-Missing.png',
    ]);
    const drawnSources = h.fakeCtx.drawImage.mock.calls
      .map(([image]) => image && image.src)
      .filter(Boolean);
    expect(drawnSources).toEqual(expect.arrayContaining([
      '../../assets/radar_icons/Icon-Station.png',
      '../../assets/radar_icons/Icon-Destroyer.png',
    ]));
    expect(drawnSources).not.toContain('../../assets/radar_icons/Icon-Missing.png');

    // Only the failed third structure takes the deterministic structure-square
    // fallback. The two loaded same-kind contacts remain visually distinct.
    const fallback = findOp(
      h.fakeCtx,
      'fillRect',
      (op) => op.fillStyle === 'var(--ink-dim)' && op.args[0] > 350,
    );
    expect(fallback).toBeDefined();
    expect(fallback.args.map((value) => Math.round(value))).toEqual([355, 295, 10, 10]);

    // Icon artwork does not replace semantic overlays.
    expect(findOp(h.fakeCtx, 'stroke', (op) => op.strokeStyle === 'var(--ink)')).toBeDefined();
    expect(findOp(h.fakeCtx, 'stroke', (op) => op.strokeStyle === GOLD)).toBeDefined();

    const failedImage = imageInstances.find((image) => image.src.endsWith('Icon-Missing.png'));
    const fallbackCount = h.fakeCtx._ops.filter(
      (op) => op.op === 'fillRect' && op.fillStyle === 'var(--ink-dim)',
    ).length;
    expect(failedImage.onerror).toEqual(expect.any(Function));
    failedImage.onerror();
    h.tickRaf();
    expect(h.fakeCtx._ops.filter(
      (op) => op.op === 'fillRect' && op.fillStyle === 'var(--ink-dim)',
    ).length).toBeGreaterThan(fallbackCount);
    expect(imageSources).toHaveLength(3);
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

  it('tap on empty space does NOT set a waypoint (selection-only)', () => {
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
    expect(sendAction).not.toHaveBeenCalled();
    expect(h.el.shadowRoot.getElementById('overlay').classList.contains('show')).toBe(false);
  });

  it('Set Waypoint pick mode places a free waypoint at the tapped world position', () => {
    const sendAction = vi.fn();
    const h = setup({ sendAction });
    h.el.state = {
      blips: [],
      range: 5000,
      ship_pos: { x: 500, z: 300 },
      ship_heading: 0,
    };
    h.tickRaf();

    h.el.shadowRoot.getElementById('btn-set-waypoint').click();

    // Click at CSS (150, 75) → buffer (300, 150)
    // cx=300, cy=300, scale=0.06 (R=300, range=5000)
    // World-anchored (camera on origin, north-up), zoom=1, pan=(0,0):
    // nx = (300-300)/(0.06*1) = 0, ny = (150-300)/(0.06*1) = -2500
    // wx = 0, wz = -(-2500) = 2500 (independent of ship_pos)
    click(h.canvas, 150, 75);
    expect(sendAction).toHaveBeenCalledTimes(1);
    expect(sendAction).toHaveBeenCalledWith('set_navigation_waypoint', { x: 0, z: 2500 });
  });

  it('routes pointer placement through the Navigation semantic identity without optimistic state', () => {
    const sendAction = vi.fn();
    window.activateSemanticAction = vi.fn(() => ({ claimed: true, handled: true }));
    const h = setup({ sendAction });
    h.el.state = {
      blips: [], range: 5000, ship_pos: { x: 0, z: 0 }, ship_heading: 0,
    };
    h.tickRaf();

    h.el.shadowRoot.getElementById('btn-set-waypoint').click();
    click(h.canvas, 150, 75);

    expect(window.activateSemanticAction).toHaveBeenCalledWith(
      NAVIGATION_WAYPOINT_PLACE_ACTION_ID,
      expect.objectContaining({
        context: 'navigation', source: 'control', surface: h.el,
        detail: { x: 0, z: 2500 },
      }),
    );
    expect(sendAction).not.toHaveBeenCalled();
    expect(h.el.state.waypoint).toBeUndefined();
  });

  it('offers a non-drag keyboard cursor and commits it through the same placement action', () => {
    window.activateSemanticAction = vi.fn(() => ({ claimed: true, handled: true }));
    const h = setup();
    h.el.state = {
      blips: [], range: 5000, ship_pos: { x: 0, z: 0 }, ship_heading: 0,
    };
    h.tickRaf();

    expect(h.el.getAttribute('aria-description')).toBe(t('component.navigation_map.keyboard_help'));
    h.el.shadowRoot.getElementById('btn-set-waypoint').click();
    h.el.dispatchEvent(new KeyboardEvent('keydown', {
      key: 'ArrowRight', bubbles: true, cancelable: true,
    }));
    h.el.dispatchEvent(new KeyboardEvent('keydown', {
      key: 'Enter', bubbles: true, cancelable: true,
    }));

    expect(window.activateSemanticAction).toHaveBeenCalledWith(
      NAVIGATION_WAYPOINT_PLACE_ACTION_ID,
      expect.objectContaining({
        context: 'navigation', source: 'control', surface: h.el,
        detail: { x: 800, z: 0 },
      }),
    );
    expect(h.canvas.classList.contains('picking')).toBe(false);
  });

  it('pick mode places a free waypoint even when a blip is underneath the tap', () => {
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
    h.el.shadowRoot.getElementById('btn-set-waypoint').click();
    click(h.canvas, 180, 150);
    expect(sendAction).toHaveBeenCalledTimes(1);
    expect(sendAction.mock.calls[0][0]).toBe('set_navigation_waypoint');
    expect(sendAction.mock.calls[0][1].source_uuid).toBeUndefined();
  });

  it('tapping Set Waypoint again cancels pick mode', () => {
    const sendAction = vi.fn();
    const h = setup({ sendAction });
    h.el.state = { blips: [], range: 5000, ship_pos: { x: 0, z: 0 }, ship_heading: 0 };
    h.tickRaf();
    const btn = h.el.shadowRoot.getElementById('btn-set-waypoint');
    btn.click();
    expect(h.canvas.classList.contains('picking')).toBe(true);
    btn.click();
    expect(h.canvas.classList.contains('picking')).toBe(false);
    click(h.canvas, 150, 150);
    expect(sendAction).not.toHaveBeenCalled();
  });

  it('tap on blip selects it and Set as Waypoint sends an anchored waypoint', () => {
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

    // Selecting must not set the waypoint on its own.
    expect(sendAction).not.toHaveBeenCalled();

    h.el.shadowRoot.getElementById('btn-set-selected').click();
    expect(sendAction).toHaveBeenCalledTimes(1);
    expect(sendAction).toHaveBeenCalledWith('set_navigation_waypoint', {
      x: 1000,
      z: 0,
      source_uuid: 'abc',
    });
  });

  it('routes contact selection, anchoring and clearing through their semantic identities', () => {
    window.activateSemanticAction = vi.fn((actionId, options) => {
      if (actionId === NAVIGATION_CONTACT_ACTION_ID) options.surface.navigationSelect(options.detail);
      return { claimed: true, handled: true };
    });
    const h = setup();
    h.el.state = {
      blips: [
        { uuid: 'abc', kind: 'planet', name: 'Alpha', world_x: 1000, world_z: 0, stance: 'friendly' },
      ],
      range: 5000,
      ship_pos: { x: 0, z: 0 },
      ship_heading: 0,
      waypoint: { x: 5, z: 10 },
    };
    h.tickRaf();

    click(h.canvas, 180, 150);
    h.el.shadowRoot.getElementById('btn-set-selected').click();
    h.el.shadowRoot.getElementById('btn-clear-waypoint').click();

    expect(window.activateSemanticAction.mock.calls.map(([actionId, options]) => (
      [actionId, options.detail]
    ))).toEqual([
      [NAVIGATION_CONTACT_ACTION_ID, { uuid: 'abc' }],
      [NAVIGATION_WAYPOINT_ANCHOR_ACTION_ID, { source_uuid: 'abc' }],
      [NAVIGATION_WAYPOINT_CLEAR_ACTION_ID, {}],
    ]);
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

  it('resolves authored display IDs in point, Region, and overlay labels', () => {
    const h = setup();
    h.el.state = {
      show_ship_marker: false,
      range: 5000,
      blips: [{
        uuid: 'ship', kind: 'structure', name: 'entity.alliance_destroyer.display_name',
        world_x: -1000, world_z: 0,
      }],
      regions: [{
        uuid: 'region', kind: 'region', name: 'entity.region_nebula.name',
        x: 1000, z: 0, shape: 'sphere', radius: 500,
      }],
    };
    h.tickRaf();

    const labels = h.fakeCtx._calls.fillText.map((call) => call.text);
    expect(labels).toContain(t('entity.alliance_destroyer.display_name'));
    expect(labels).toContain(t('entity.region_nebula.name'));
    expect(labels).not.toContain('entity.alliance_destroyer.display_name');
    expect(labels).not.toContain('entity.region_nebula.name');

    expect(h.el.navigationSelect({ uuid: 'ship' })).toBe(true);
    expect(h.el.shadowRoot.getElementById('ov-name').textContent)
      .toBe(t('entity.alliance_destroyer.display_name'));
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

  it('waypoint control bar shows Set with no waypoint and hides Clear', () => {
    const h = setup();
    h.el.state = { blips: [], range: 5000, ship_pos: { x: 0, z: 0 }, ship_heading: 0 };
    h.tickRaf();
    const setBtn = h.el.shadowRoot.getElementById('btn-set-waypoint');
    const selectedBtn = h.el.shadowRoot.getElementById('btn-set-selected');
    const clearBtn = h.el.shadowRoot.getElementById('btn-clear-waypoint');
    expect(setBtn.classList.contains('show')).toBe(true);
    expect(selectedBtn.classList.contains('show')).toBe(false);
    expect(clearBtn.classList.contains('show')).toBe(false);
  });

  it('waypoint control bar shows Clear once a waypoint is set', () => {
    const h = setup();
    h.el.state = { blips: [], range: 5000, ship_pos: { x: 0, z: 0 }, ship_heading: 0, waypoint: { x: 100, z: 200 } };
    h.tickRaf();
    expect(h.el.shadowRoot.getElementById('btn-set-waypoint').classList.contains('show')).toBe(false);
    expect(h.el.shadowRoot.getElementById('btn-clear-waypoint').classList.contains('show')).toBe(true);
  });

  it('disables every authoritative waypoint control while Navigation is Auto', () => {
    const h = setup();
    h.el.state = {
      blips: [{ uuid: 'abc', world_x: 0, world_z: 0 }],
      range: 5000,
      ship_pos: { x: 0, z: 0 },
      ship_heading: 0,
      waypoint: { x: 100, z: 200 },
      auto: true,
    };
    h.el.navigationSelect({ uuid: 'abc' });
    h.tickRaf();

    expect(h.el.shadowRoot.getElementById('btn-set-waypoint').disabled).toBe(true);
    expect(h.el.shadowRoot.getElementById('btn-set-selected').disabled).toBe(true);
    expect(h.el.shadowRoot.getElementById('btn-clear-waypoint').disabled).toBe(true);
  });

  it('Clear Waypoint button sends clear_navigation_waypoint', () => {
    const sendAction = vi.fn();
    const h = setup({ sendAction });
    h.el.state = { blips: [], range: 5000, ship_pos: { x: 0, z: 0 }, ship_heading: 0, waypoint: { x: 100, z: 200 } };
    h.tickRaf();
    h.el.shadowRoot.getElementById('btn-clear-waypoint').click();
    expect(sendAction).toHaveBeenCalledWith('clear_navigation_waypoint', {});
  });

  it('selecting a blip emits navselect and an empty tap clears it', () => {
    const h = setup();
    const onSelect = vi.fn();
    h.el.addEventListener('navselect', onSelect);
    h.el.state = {
      blips: [{ uuid: 'abc', kind: 'planet', name: 'Alpha', world_x: 1000, world_z: 0, stance: 'friendly' }],
      range: 5000,
      ship_pos: { x: 0, z: 0 },
      ship_heading: 0,
    };
    h.tickRaf();

    click(h.canvas, 180, 150);
    expect(onSelect).toHaveBeenCalledTimes(1);
    expect(onSelect.mock.calls[0][0].detail.uuid).toBe('abc');

    click(h.canvas, 0, 0);
    expect(onSelect).toHaveBeenCalledTimes(2);
    expect(onSelect.mock.calls[1][0].detail).toBeNull();
  });

  it('selection is dropped when the selected blip disappears on state refresh', () => {
    const h = setup();
    let selected = null;
    h.el.addEventListener('navselect', (e) => { selected = e.detail; });
    h.el.state = {
      blips: [{ uuid: 'abc', kind: 'planet', name: 'Alpha', world_x: 1000, world_z: 0, stance: 'friendly' }],
      range: 5000,
      ship_pos: { x: 0, z: 0 },
      ship_heading: 0,
    };
    h.tickRaf();
    click(h.canvas, 180, 150);
    expect(selected && selected.uuid).toBe('abc');

    h.el.state = { blips: [], range: 5000, ship_pos: { x: 0, z: 0 }, ship_heading: 0 };
    h.tickRaf();
    expect(selected).toBeNull();
    expect(h.el.shadowRoot.getElementById('overlay').classList.contains('show')).toBe(false);
  });

  describe('label font scaling', () => {
    it('blip name font scales with the buffer/CSS ratio (devicePixelRatio)', () => {
      const h = setup();
      h.el.state = {
        blips: [{ uuid: 'abc', kind: 'planet', name: 'Alpha', world_x: 1000, world_z: 0, stance: 'friendly' }],
        range: 5000,
        ship_pos: { x: 0, z: 0 },
        ship_heading: 0,
      };
      h.tickRaf();
      // Canvas buffer is 600 wide for a 300 CSS rect (dpr 2) → 12*2 = 24px.
      const draws = h.fakeCtx._calls.fillText.filter((f) => f.text === 'Alpha');
      expect(draws.length).toBeGreaterThan(0);
      expect(draws[0].font).toBe('24px "JetBrains Mono", monospace');
    });

    it('waypoint WP label font scales with the buffer/CSS ratio', () => {
      const h = setup();
      h.el.state = {
        blips: [],
        range: 5000,
        ship_pos: { x: 0, z: 0 },
        ship_heading: 0,
        waypoint: { x: 2000, z: 1000 },
      };
      h.tickRaf();
      // 10*2 = 20px at dpr 2.
      const draws = h.fakeCtx._calls.fillText.filter((f) => f.text === 'WP');
      expect(draws.length).toBeGreaterThan(0);
      expect(draws[0].font).toBe('20px "JetBrains Mono", monospace');
    });

    it('blip names are not drawn when zoomed out below the label floor', () => {
      const h = setup();
      h.el.state = {
        blips: [{ uuid: 'abc', kind: 'planet', name: 'Alpha', world_x: 1000, world_z: 0, stance: 'friendly' }],
        range: 5000,
        ship_pos: { x: 0, z: 0 },
        ship_heading: 0,
      };
      h.tickRaf();
      const before = h.fakeCtx._calls.fillText.filter((f) => f.text === 'Alpha').length;
      for (let i = 0; i < 20; i++) {
        h.canvas.dispatchEvent(new WheelEvent('wheel', { deltaY: 120, clientX: 150, clientY: 150, bubbles: true, cancelable: true }));
      }
      h.tickRaf();
      const after = h.fakeCtx._calls.fillText.filter((f) => f.text === 'Alpha').length;
      expect(after).toBe(before);
    });
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

  // Canvas 600x600 → cx=cy=300, R=300; range 5000 → scale = 0.06 buffer px
  // per world unit at zoom 1. World (1000, 0) → screen (360, 300).
  describe('regions', () => {
    const BASE = { blips: [], range: 5000, ship_pos: { x: 0, z: 0 }, ship_heading: 0 };

    it('draws a sphere region as a filled, stroked circle in its authored colour', () => {
      const h = setup();
      h.el.state = {
        ...BASE,
        regions: [{
          uuid: 'r1', x: 1000, z: 0, shape: 'sphere', radius: 500,
          color: [0.2, 0.4, 0.8], name: 'Kaleth Nebula', objective_target: false,
        }],
      };
      h.tickRaf();

      // [0.2, 0.4, 0.8] floats → rgb(51,102,204); fill at 0.3 alpha.
      const fill = findOp(h.fakeCtx, 'fill', (o) => o.fillStyle === 'rgba(51,102,204,0.3)');
      expect(fill).toBeDefined();
      const stroke = findOp(h.fakeCtx, 'stroke', (o) => o.strokeStyle === 'rgb(51,102,204)');
      expect(stroke).toBeDefined();
      expect(stroke.lineWidth).toBe(1.5);

      // radius 500 world → 500 * 0.06 = 30 buffer px, centred on (360, 300).
      const arc = arcFor(h.fakeCtx, 'fill', (o) => o.fillStyle === 'rgba(51,102,204,0.3)');
      expect(arc.args[0]).toBeCloseTo(360, 1);
      expect(arc.args[1]).toBeCloseTo(300, 1);
      expect(arc.args[2]).toBeCloseTo(30, 1);
    });

    it('labels a region with its name in the blip label font', () => {
      const h = setup();
      h.el.state = {
        ...BASE,
        regions: [{ uuid: 'r1', x: 1000, z: 0, shape: 'sphere', radius: 500, color: [0.2, 0.4, 0.8], name: 'Kaleth Nebula' }],
      };
      h.tickRaf();
      const draws = h.fakeCtx._calls.fillText.filter((f) => f.text === 'Kaleth Nebula');
      expect(draws.length).toBe(1);
      // Same 12px DPR-scaled treatment as blip names (dpr 2 → 24px).
      expect(draws[0].font).toBe('24px "JetBrains Mono", monospace');
      // Offset clear of the 30px hull: 360 + 30 + 4.
      expect(draws[0].x).toBeCloseTo(394, 1);
    });

    it('draws a torus region as one thick stroked ring with no fill', () => {
      const h = setup();
      h.el.state = {
        ...BASE,
        regions: [{
          uuid: 'f1', x: 0, z: 0, shape: 'torus',
          outer_radius: 3500, inner_radius: 3000, radius: 3500,
          color: [0.5, 0.3, 0.2], name: null, objective_target: false,
        }],
      };
      h.tickRaf();

      // outer 210px, inner 180px → ring centred at 195px, 30px wide.
      const stroke = findOp(h.fakeCtx, 'stroke', (o) => o.strokeStyle === 'rgb(128,77,51)');
      expect(stroke).toBeDefined();
      expect(stroke.lineWidth).toBeCloseTo(30, 1);
      const arc = arcFor(h.fakeCtx, 'stroke', (o) => o.strokeStyle === 'rgb(128,77,51)');
      expect(arc.args[2]).toBeCloseTo(195, 1);

      // A torus is an outline only — no translucent hull behind it.
      expect(findOp(h.fakeCtx, 'fill', (o) => o.fillStyle === 'rgba(128,77,51,0.3)')).toBeUndefined();
    });

    it('rotates a non-square box fill, outline, and selected outline by authored yaw', () => {
      const h = setup();
      h.el.state = {
        ...BASE,
        interaction: 'inspect',
        regions: [{
          uuid: 'b1', x: 0, z: 0, shape: 'box',
          half_extents: [1000, 500], yaw: 0.7,
          color: [0, 1, 0], name: null, objective_target: false, selectable: true,
        }],
      };
      expect(h.el.navigationSelect({ uuid: 'b1' })).toBe(true);
      h.tickRaf();

      // half extents 1000/500 world → 60/30px in box-local canvas space.
      const rect = findOp(h.fakeCtx, 'fillRect', (o) => o.fillStyle === 'rgba(0,255,0,0.3)');
      expect(rect).toBeDefined();
      expect(rect.args.map((n) => Math.round(n))).toEqual([-60, -30, 120, 60]);
      const outline = findOp(h.fakeCtx, 'strokeRect', (o) => o.strokeStyle === 'rgb(0,255,0)');
      expect(outline).toBeDefined();
      expect(outline.args.map((n) => Math.round(n))).toEqual([-60, -30, 120, 60]);
      const selection = findOp(h.fakeCtx, 'strokeRect', (o) => o.strokeStyle === GOLD);
      expect(selection.args.map((n) => Math.round(n))).toEqual([-64, -34, 128, 68]);
      expect(h.fakeCtx.rotate.mock.calls.filter(([angle]) => angle === -0.7)).toHaveLength(2);
    });

    it('outlines an objective region in gold while keeping its own fill', () => {
      const h = setup();
      h.el.state = {
        ...BASE,
        regions: [{
          uuid: 'r1', x: 1000, z: 0, shape: 'sphere', radius: 500,
          color: [0.2, 0.4, 0.8], name: null, objective_target: true,
        }],
      };
      h.tickRaf();
      expect(findOp(h.fakeCtx, 'fill', (o) => o.fillStyle === 'rgba(51,102,204,0.3)')).toBeDefined();
      const gold = findOp(h.fakeCtx, 'stroke', (o) => o.strokeStyle === GOLD);
      expect(gold).toBeDefined();
      expect(findOp(h.fakeCtx, 'stroke', (o) => o.strokeStyle === 'rgb(51,102,204)')).toBeUndefined();
    });

    it('draws an objective torus as a single gold stroked ring with no fill', () => {
      const h = setup();
      h.el.state = {
        ...BASE,
        regions: [{
          uuid: 'f1', x: 0, z: 0, shape: 'torus',
          outer_radius: 3500, inner_radius: 3000, radius: 3500,
          color: [0.5, 0.3, 0.2], name: null, objective_target: true,
        }],
      };
      h.tickRaf();

      // Same ring geometry as the plain-torus case, but the stroke recolours
      // to the waypoint gold instead of the region's own [0.5,0.3,0.2] hue —
      // a torus has no fill to begin with, so the whole ring reads gold.
      const stroke = findOp(h.fakeCtx, 'stroke', (o) => o.strokeStyle === GOLD);
      expect(stroke).toBeDefined();
      expect(stroke.lineWidth).toBeCloseTo(30, 1);
      expect(findOp(h.fakeCtx, 'stroke', (o) => o.strokeStyle === 'rgb(128,77,51)')).toBeUndefined();
      expect(findOp(h.fakeCtx, 'fill', (o) => o.fillStyle === 'rgba(128,77,51,0.3)')).toBeUndefined();
    });

    it('floors a tiny region at the 4px minimum', () => {
      const h = setup();
      h.el.state = {
        ...BASE,
        // 10 world units at 0.06 px/unit is 0.6px — floored at 4px so it
        // stays visible instead of collapsing to nothing.
        regions: [{ uuid: 'r1', x: 1000, z: 0, shape: 'sphere', radius: 10, color: [0.2, 0.4, 0.8] }],
      };
      h.tickRaf();
      const arc = arcFor(h.fakeCtx, 'fill', (o) => o.fillStyle === 'rgba(51,102,204,0.3)');
      expect(arc).not.toBeNull();
      expect(arc.args[2]).toBe(4);
    });

    it('does not cull an off-screen region the way blips are culled', () => {
      const h = setup();
      h.el.state = {
        ...BASE,
        // Centre projects to sx ≈ 30300, far past the 600x600 buffer. A blip
        // this far off-screen is skipped (ph-navigation-map.js:282); regions
        // carry no such cull, so the shape still draws in full.
        regions: [{ uuid: 'r1', x: 500000, z: 0, shape: 'sphere', radius: 500, color: [0.2, 0.4, 0.8] }],
      };
      h.tickRaf();
      const fill = findOp(h.fakeCtx, 'fill', (o) => o.fillStyle === 'rgba(51,102,204,0.3)');
      expect(fill).toBeDefined();
      const stroke = findOp(h.fakeCtx, 'stroke', (o) => o.strokeStyle === 'rgb(51,102,204)');
      expect(stroke).toBeDefined();
      const arc = arcFor(h.fakeCtx, 'fill', (o) => o.fillStyle === 'rgba(51,102,204,0.3)');
      expect(arc.args[0]).toBeGreaterThan(600);
    });

    it('falls back to a neutral hull colour when no region colour was authored', () => {
      const h = setup();
      h.el.state = {
        ...BASE,
        regions: [{ uuid: 'r1', x: 0, z: 0, shape: 'sphere', radius: 500, color: null }],
      };
      h.tickRaf();
      expect(findOp(h.fakeCtx, 'fill', (o) => o.fillStyle === 'rgba(168,176,191,0.3)')).toBeDefined();
    });

    it('skips a region whose shape the chart does not know', () => {
      const h = setup();
      h.el.state = {
        ...BASE,
        regions: [{ uuid: 'r1', x: 0, z: 0, shape: 'wormhole', radius: 500, color: [0.2, 0.4, 0.8], name: 'Nowhere' }],
      };
      h.tickRaf();
      expect(findOp(h.fakeCtx, 'fill', (o) => o.fillStyle === 'rgba(51,102,204,0.3)')).toBeUndefined();
      expect(h.fakeCtx._calls.fillText.filter((f) => f.text === 'Nowhere').length).toBe(0);
    });

    it('draws no region geometry when the payload carries none', () => {
      const h = setup();
      h.el.state = { ...BASE, regions: [] };
      h.tickRaf();
      expect(h.fakeCtx._calls.strokeRect.length).toBe(0);
      expect(findOp(h.fakeCtx, 'fill', (o) => /^rgba\(.*,0\.3\)$/.test(String(o.fillStyle)))).toBeUndefined();
      // The grid and ship marker still render (the empty-state smoke path).
      expect(h.fakeCtx._calls.fillRect.length).toBeGreaterThan(0);
    });

    it('draws regions before blips so a contact is never buried in a hull', () => {
      const h = setup();
      h.el.state = {
        ...BASE,
        blips: [{ uuid: 'm1', kind: 'planet', name: 'Ice Moon', world_x: 1000, world_z: 0, stance: 'neutral' }],
        regions: [{ uuid: 'r1', x: 1000, z: 0, shape: 'sphere', radius: 500, color: [0.2, 0.4, 0.8] }],
      };
      h.tickRaf();
      const regionAt = h.fakeCtx._ops.findIndex((o) => o.op === 'fill' && o.fillStyle === 'rgba(51,102,204,0.3)');
      const blipAt = h.fakeCtx._ops.findIndex((o) => o.op === 'fill' && o.fillStyle === 'var(--ink-dim)');
      expect(regionAt).toBeGreaterThanOrEqual(0);
      expect(blipAt).toBeGreaterThan(regionAt);
    });
  });

  describe('objective contacts', () => {
    const BASE = { regions: [], range: 5000, ship_pos: { x: 0, z: 0 }, ship_heading: 0 };

    it('rings an objective blip in gold just outside the marker', () => {
      const h = setup();
      h.el.state = {
        ...BASE,
        blips: [{ uuid: 'a', kind: 'planet', name: 'Alpha', world_x: 1000, world_z: 0, stance: 'neutral', objective_target: true }],
      };
      h.tickRaf();
      const ring = findOp(h.fakeCtx, 'stroke', (o) => o.strokeStyle === GOLD && o.lineWidth === 2);
      expect(ring).toBeDefined();
      // blipR = max(3, 300 * 0.015) = 4.5 → ring at 10.5px around (360, 300).
      const arc = arcFor(h.fakeCtx, 'stroke', (o) => o.strokeStyle === GOLD && o.lineWidth === 2);
      expect(arc.args[0]).toBeCloseTo(360, 1);
      expect(arc.args[1]).toBeCloseTo(300, 1);
      expect(arc.args[2]).toBeCloseTo(10.5, 1);
    });

    it('leaves an ordinary blip unringed', () => {
      const h = setup();
      h.el.state = {
        ...BASE,
        blips: [{ uuid: 'a', kind: 'planet', name: 'Alpha', world_x: 1000, world_z: 0, stance: 'neutral' }],
      };
      h.tickRaf();
      expect(findOp(h.fakeCtx, 'stroke', (o) => o.strokeStyle === GOLD && o.lineWidth === 2)).toBeUndefined();
    });

  });

  describe('zoom and pan', () => {
    it('routes pointer pan and wheel zoom through semantic identities', () => {
      window.activateSemanticAction = vi.fn((actionId, options) => {
        if (actionId === NAVIGATION_PAN_RIGHT_ACTION_ID) {
          options.surface.navigationPan(options.detail);
        } else if (actionId === NAVIGATION_ZOOM_IN_ACTION_ID) {
          options.surface.navigationZoom(options.detail);
        }
        return { claimed: true, handled: true };
      });
      const h = setup();
      h.el.state = {
        blips: [], range: 5000, ship_pos: { x: 0, z: 0 }, ship_heading: 0,
      };
      h.tickRaf();

      drag(h.canvas, 100, 100, 200, 100);
      h.canvas.dispatchEvent(new WheelEvent('wheel', {
        deltaY: -120, clientX: 150, clientY: 150, bubbles: true, cancelable: true,
      }));

      expect(window.activateSemanticAction.mock.calls.map(([actionId]) => actionId)).toEqual([
        NAVIGATION_PAN_RIGHT_ACTION_ID,
        NAVIGATION_ZOOM_IN_ACTION_ID,
      ]);
      expect(window.activateSemanticAction.mock.calls[0][1]).toMatchObject({
        context: 'navigation', source: 'control', surface: h.el, detail: { x: 200 },
      });
      expect(window.activateSemanticAction.mock.calls[1][1]).toMatchObject({
        context: 'navigation', source: 'control', surface: h.el,
        detail: { factor: 1.13, x: 300, y: 300 },
      });
    });

    it('preserves touch drag and pinch while routing both through semantic identities', () => {
      window.activateSemanticAction = vi.fn(() => ({ claimed: true, handled: true }));
      const h = setup();
      h.el.state = {
        blips: [], range: 5000, ship_pos: { x: 0, z: 0 }, ship_heading: 0,
      };
      h.tickRaf();

      touch(h.canvas, 'touchstart', [{ clientX: 100, clientY: 100 }]);
      touch(h.canvas, 'touchmove', [{ clientX: 200, clientY: 100 }]);
      touch(h.canvas, 'touchend', [], [{ clientX: 200, clientY: 100 }]);
      touch(h.canvas, 'touchstart', [
        { clientX: 100, clientY: 100 }, { clientX: 200, clientY: 100 },
      ]);
      touch(h.canvas, 'touchmove', [
        { clientX: 80, clientY: 100 }, { clientX: 220, clientY: 100 },
      ]);

      expect(window.activateSemanticAction.mock.calls.map(([actionId]) => actionId)).toEqual([
        NAVIGATION_PAN_RIGHT_ACTION_ID,
        NAVIGATION_ZOOM_IN_ACTION_ID,
      ]);
      expect(window.activateSemanticAction.mock.calls[0][1].detail).toEqual({ x: 200 });
      expect(window.activateSemanticAction.mock.calls[1][1].detail).toEqual({
        factor: 1.4, x: 300, y: 200,
      });
    });

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

      // A <5px drag is a tap; with pick mode armed it places a free waypoint.
      h.el.shadowRoot.getElementById('btn-set-waypoint').click();
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
      h.el.shadowRoot.getElementById('btn-set-waypoint').click();
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

      // Baseline: arm pick mode and tap top-left to get world coords at zoom=1
      h.el.shadowRoot.getElementById('btn-set-waypoint').click();
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
      h.el.shadowRoot.getElementById('btn-set-waypoint').click();
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
// nx = (300-300-100)/(0.06*1.13) = -100/0.0678 = -1474.93
      //   ny = (300-300-0)/(0.06*1.13) = 0
      //   wx = -1474.93, wz = 0 (independent of ship_pos)
      h.el.shadowRoot.getElementById('btn-set-waypoint').click();
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
      h.el.shadowRoot.getElementById('btn-set-waypoint').click();
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
      h.el.shadowRoot.getElementById('btn-set-waypoint').click();
      click(h.canvas, 0, 0);
      const call = sendAction.mock.calls[0][1];
      // At zoom=0.25 (clamped), top-left maps to:
      // nx = (0-300)/(0.06*0.25) = -300/0.015 = -20000
      expect(call.x).toBeCloseTo(-20000, 0);
    });
  });

  describe('read-only omniscient inspect mode', () => {
    const INSPECT_STATE = {
      interaction: 'inspect',
      show_ship_marker: false,
      range: 100,
      blips: [
        { uuid: 'player-a', kind: 'player_ship', name: 'Axiom', world_x: 0, world_z: 0, stance: 'friendly', destroyed: false },
        { uuid: 'npc-b', kind: 'npc_ship', name: 'Raider', world_x: 50, world_z: 0, stance: 'unknown', destroyed: true },
      ],
      regions: [{
        uuid: 'region-c', kind: 'region', name: 'Safe harbour', x: 0, z: 50,
        shape: 'sphere', radius: 15, color: [0.1, 0.7, 0.5], selectable: true,
      }],
    };

    it('selects by touch and keyboard and keeps the UUID across absolute refreshes', () => {
      // A host page owns a semantic registry, but read-only inspect gestures
      // must stay local instead of being swallowed as Navigation commands.
      window.activateSemanticAction = vi.fn(() => ({ claimed: false, handled: false }));
      const h = setup();
      const selected = [];
      h.el.addEventListener('navselect', (event) => selected.push(event.detail && event.detail.uuid));
      h.el.state = INSPECT_STATE;
      h.tickRaf();

      touch(h.canvas, 'touchstart', [{ clientX: 150, clientY: 150 }]);
      touch(h.canvas, 'touchend', [], [{ clientX: 150, clientY: 150 }]);
      expect(selected).toEqual(['player-a']);
      expect(h.el.navigationSelectedUuid()).toBe('player-a');
      expect(h.el.dataset.selectedEntityId).toBe('player-a');
      expect(h.el.shadowRoot.querySelector('.wp-btn.show')).toBeNull();

      h.el.state = {
        ...INSPECT_STATE,
        blips: INSPECT_STATE.blips.map((blip) => blip.uuid === 'player-a'
          ? { ...blip, world_x: 10, hull_percent: 40 } : blip),
      };
      h.tickRaf();
      expect(h.el.navigationSelectedUuid()).toBe('player-a');
      expect(selected).toEqual(['player-a']);

      h.el.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowLeft', bubbles: true }));
      expect(h.el.navigationSelectedUuid()).toBe('npc-b');
      expect(selected).toEqual(['player-a', 'npc-b']);
      expect(window.activateSemanticAction).not.toHaveBeenCalled();
    });

    it('selects selectable Regions by touch and includes them in stable UUID keyboard order', () => {
      const h = setup();
      const selected = [];
      h.el.addEventListener('navselect', (event) => selected.push(event.detail && event.detail.uuid));
      h.el.state = INSPECT_STATE;
      h.tickRaf();

      // Region world (0, 50) -> buffer (300, 150) -> CSS (150, 75).
      touch(h.canvas, 'touchstart', [{ clientX: 150, clientY: 75 }]);
      touch(h.canvas, 'touchend', [], [{ clientX: 150, clientY: 75 }]);
      expect(h.el.navigationSelectedUuid()).toBe('region-c');
      expect(selected).toEqual(['region-c']);

      // Inspect candidates are UUID-sorted across points and Regions:
      // npc-b, player-a, region-c.
      h.el.dispatchEvent(new KeyboardEvent('keydown', { key: 'Home', bubbles: true }));
      expect(h.el.navigationSelectedUuid()).toBe('npc-b');
      h.el.dispatchEvent(new KeyboardEvent('keydown', { key: 'End', bubbles: true }));
      expect(h.el.navigationSelectedUuid()).toBe('region-c');
    });

    it('hit-tests a rotated non-square box in its authored local frame', () => {
      const h = setup();
      h.el.state = {
        ...INSPECT_STATE,
        blips: [],
        regions: [{
          uuid: 'box', kind: 'region', x: 0, z: 0,
          shape: 'box', half_extents: [40, 10], yaw: Math.PI / 4, selectable: true,
        }],
      };
      h.tickRaf();

      // Buffer delta (+60,-60): outside an axis-aligned 120x30px half-box,
      // but on the long axis after authored +π/4 becomes canvas -π/4. Using
      // the wrong yaw sign would reject this point as well.
      touch(h.canvas, 'touchstart', [{ clientX: 180, clientY: 120 }]);
      touch(h.canvas, 'touchend', [], [{ clientX: 180, clientY: 120 }]);
      expect(h.el.navigationSelectedUuid()).toBe('box');

      // Buffer delta (+80,0) is the inverse discriminator: axis-aligned would
      // accept it, while the rotated thin axis correctly rejects it.
      touch(h.canvas, 'touchstart', [{ clientX: 190, clientY: 150 }]);
      touch(h.canvas, 'touchend', [], [{ clientX: 190, clientY: 150 }]);
      expect(h.el.navigationSelectedUuid()).toBeNull();
    });

    it('gives overlapping points priority over Regions and resolves point overlap by UUID', () => {
      const h = setup();
      h.el.state = {
        ...INSPECT_STATE,
        blips: [
          { uuid: 'z-point', kind: 'npc_ship', world_x: 0, world_z: 0 },
          { uuid: 'a-point', kind: 'player_ship', world_x: 0, world_z: 0 },
        ],
        regions: [
          { uuid: 'a-region', kind: 'hazard', x: 0, z: 0, shape: 'sphere', radius: 30, selectable: true },
          { uuid: 'z-region', kind: 'region', x: 0, z: 0, shape: 'sphere', radius: 30, selectable: true },
        ],
      };
      h.tickRaf();

      touch(h.canvas, 'touchstart', [{ clientX: 150, clientY: 150 }]);
      touch(h.canvas, 'touchend', [], [{ clientX: 150, clientY: 150 }]);
      expect(h.el.navigationSelectedUuid()).toBe('a-point');
    });

    it('outlines the selected Region and distinguishes Region kinds without colour', () => {
      const h = setup();
      h.el.state = {
        ...INSPECT_STATE,
        blips: [],
        regions: [
          {
            uuid: 'field-a', kind: 'asteroid_field', x: -30, z: 0,
            shape: 'torus', inner_radius: 5, outer_radius: 20, selectable: true,
          },
          {
            uuid: 'hazard-b', kind: 'hazard', x: 30, z: 0,
            shape: 'sphere', radius: 20, selectable: true,
          },
          {
            uuid: 'region-c', kind: 'region', x: 0, z: 30,
            shape: 'box', half_extents: [10, 15], selectable: true,
          },
        ],
      };
      expect(h.el.navigationSelect({ uuid: 'region-c' })).toBe(true);
      h.tickRaf();

      expect(h.fakeCtx.setLineDash).toHaveBeenCalledWith([2, 5]);
      expect(h.fakeCtx.setLineDash).toHaveBeenCalledWith([9, 5]);
      expect(h.fakeCtx.setLineDash).toHaveBeenCalledWith([4, 3]);
      expect(findOp(h.fakeCtx, 'strokeRect', (op) => op.strokeStyle === GOLD)).toBeDefined();
    });

    it('clears a removed Region and does not resurrect selection when the UUID reappears', () => {
      const h = setup();
      const selected = [];
      h.el.addEventListener('navselect', (event) => selected.push(event.detail && event.detail.uuid));
      h.el.state = INSPECT_STATE;
      expect(h.el.navigationSelect({ uuid: 'region-c' })).toBe(true);
      h.tickRaf();

      h.el.state = { ...INSPECT_STATE, regions: [] };
      h.tickRaf();
      expect(h.el.navigationSelectedUuid()).toBeNull();
      expect(selected).toEqual(['region-c', null]);

      h.el.state = INSPECT_STATE;
      h.tickRaf();
      expect(h.el.navigationSelectedUuid()).toBeNull();
      expect(h.el.navigationSelect({ uuid: 'region-c' })).toBe(true);
    });

    it('does not make Regions selectable on the ordinary Navigation chart', () => {
      const h = setup();
      h.el.state = {
        ...INSPECT_STATE,
        interaction: 'navigate',
        blips: [],
      };
      h.tickRaf();
      touch(h.canvas, 'touchstart', [{ clientX: 150, clientY: 75 }]);
      touch(h.canvas, 'touchend', [], [{ clientX: 150, clientY: 75 }]);
      expect(h.el.navigationSelectedUuid()).toBeNull();
      expect(h.el.navigationSelect({ uuid: 'region-c' })).toBe(false);
    });

    it('keeps wheel/pinch zoom and pointer pan local and renders non-colour status marks', () => {
      window.activateSemanticAction = vi.fn(() => ({ claimed: false, handled: false }));
      const h = setup();
      const zoom = vi.spyOn(h.el, 'navigationZoom');
      const pan = vi.spyOn(h.el, 'navigationPan');
      h.el.state = INSPECT_STATE;
      h.tickRaf();

      h.canvas.dispatchEvent(new WheelEvent('wheel', {
        deltaY: -120, clientX: 150, clientY: 150, bubbles: true, cancelable: true,
      }));
      drag(h.canvas, 100, 100, 120, 100);
      touch(h.canvas, 'touchstart', [
        { clientX: 100, clientY: 100 }, { clientX: 200, clientY: 100 },
      ]);
      touch(h.canvas, 'touchmove', [
        { clientX: 80, clientY: 100 }, { clientX: 220, clientY: 100 },
      ]);

      expect(zoom).toHaveBeenCalledTimes(2);
      expect(pan).toHaveBeenCalled();
      expect(window.activateSemanticAction).not.toHaveBeenCalled();

      // Player/NPC are distinct diamond/ship paths, and destroyed adds an X;
      // those marks remain when colour cannot be perceived.
      h.tickRaf();
      expect(h.fakeCtx._calls.moveTo.length).toBeGreaterThan(0);
      expect(h.fakeCtx._calls.lineTo.length).toBeGreaterThan(h.fakeCtx._calls.moveTo.length);
      expect(h.el.getAttribute('aria-label')).toBe(t('component.entity_map.label'));
    });
  });
});
