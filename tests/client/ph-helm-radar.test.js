// @vitest-environment jsdom
import { t } from '../../gui/strings.js';
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import '../../gui/components/ph-helm-radar.js';

let rafCb;
let rafIdCounter;
let origRAF;
let origCARAF;
let origRO;
let origGetContext;

function makeFakeCtx() {
  const ctx = {
    fillStyle: '',
    font: '',
    fillRect: vi.fn(),
    beginPath: vi.fn(),
    arc: vi.fn(),
    fill: vi.fn(),
    fillText: vi.fn(),
    drawImage: vi.fn(),
  };
  return ctx;
}

function mockRAF() {
  rafCb = null;
  rafIdCounter = 0;
  origRAF = window.requestAnimationFrame;
  origCARAF = window.cancelAnimationFrame;
  window.requestAnimationFrame = vi.fn((cb) => { rafCb = cb; return ++rafIdCounter; });
  window.cancelAnimationFrame = vi.fn();
}

function restoreRAF() {
  if (origRAF) window.requestAnimationFrame = origRAF;
  if (origCARAF) window.cancelAnimationFrame = origCARAF;
}

function setup(opts) {
  if (opts && opts.sendAction) {
    window.sendAction = opts.sendAction;
  }
  document.body.innerHTML = '<ph-helm-radar id="test-el"></ph-helm-radar>';
  const el = document.getElementById('test-el');
  return { el };
}

describe('PhHelmRadar', () => {
  beforeEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
    mockRAF();
    origRO = window.ResizeObserver;
    window.ResizeObserver = function () {
      return { observe: vi.fn(), disconnect: vi.fn() };
    };
    origGetContext = HTMLCanvasElement.prototype.getContext;
    HTMLCanvasElement.prototype.getContext = function () { return makeFakeCtx(); };
  });

  afterEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
    restoreRAF();
    if (origRO) window.ResizeObserver = origRO;
    if (origGetContext) HTMLCanvasElement.prototype.getContext = origGetContext;
  });

  it('is defined and registered as a custom element', () => {
    expect(customElements.get('ph-helm-radar')).toBeDefined();
  });

  it('creates a shadow root with inner ph-radar, SVG overlay, and ON SCREEN button', () => {
    const { el } = setup();
    expect(el.shadowRoot).toBeDefined();
    expect(el.shadowRoot.getElementById('inner-radar')).toBeDefined();
    expect(el.shadowRoot.getElementById('arc-port')).toBeDefined();
    expect(el.shadowRoot.getElementById('arc-stbd')).toBeDefined();
    expect(el.shadowRoot.getElementById('on-screen-btn')).toBeDefined();
  });

  it('passes base state through to inner ph-radar', () => {
    const { el } = setup();
    const inner = el.shadowRoot.getElementById('inner-radar');

    el.state = {
      blips: [{ uuid: 'a', bearing_deg: 0, range: 500 }],
      range: 1000,
      ship_heading: 90,
      on_screen_active: false,
    };

    expect(inner.state).toEqual({
      blips: [{ uuid: 'a', bearing_deg: 0, range: 500 }],
      range: 1000,
      ship_heading: 90,
      config: {},
    });
  });

  it('passes through config to inner ph-radar', () => {
    const { el } = setup();
    const inner = el.shadowRoot.getElementById('inner-radar');
    el.state = { config: { max_range: 5000 } };
    expect(inner.state.config).toEqual({ max_range: 5000 });
  });

  it('ON SCREEN button dispatches sendAction with set_radar_view when clicked', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });

    const btn = el.shadowRoot.getElementById('on-screen-btn');
    btn.click();

    expect(sendAction).toHaveBeenCalledWith('set_radar_view', {});
  });

  it('ON SCREEN button shows active class when on_screen_active is true', () => {
    const { el } = setup();
    const btn = el.shadowRoot.getElementById('on-screen-btn');

    expect(btn.classList.contains('active')).toBe(false);

    el.state = { on_screen_active: true };
    expect(btn.classList.contains('active')).toBe(true);

    el.state = { on_screen_active: false };
    expect(btn.classList.contains('active')).toBe(false);
  });

  it('thrust arcs update SVG path attributes from engine_port_thrust and engine_stbd_thrust', () => {
    const { el } = setup();
    const arcPort = el.shadowRoot.getElementById('arc-port');
    const arcStbd = el.shadowRoot.getElementById('arc-stbd');

    el.state = { engine_port_thrust: 0.5, engine_stbd_thrust: 1.0 };

    const portD = arcPort.getAttribute('d');
    const stbdD = arcStbd.getAttribute('d');
    expect(portD).toBeTruthy();
    expect(portD).toContain('A');
    expect(stbdD).toBeTruthy();
    expect(stbdD).toContain('A');
    expect(arcPort.style.opacity).toBe(String(0.2 + 0.8 * 0.5));
    expect(arcStbd.style.opacity).toBe(String(1.0));
  });

  it('zero thrust clears arc path', () => {
    const { el } = setup();
    const arcPort = el.shadowRoot.getElementById('arc-port');

    el.state = { engine_port_thrust: 0 };
    expect(arcPort.getAttribute('d')).toBe('');
  });

  it('clamps thrust arcs to [0, 1]', () => {
    const { el } = setup();
    const arcPort = el.shadowRoot.getElementById('arc-port');

    el.state = { engine_port_thrust: 2.5 };
    const d = arcPort.getAttribute('d');
    expect(d).toBeTruthy();

    el.state = { engine_port_thrust: -0.5 };
    expect(arcPort.getAttribute('d')).toBe('');
  });
  // ── Hostile weapon-arc overlay (issue #874) ─────────────────────────────
  //
  // AC4, JS half: the component renders EXACTLY the sectors it is handed and
  // recomputes nothing. The wire sectors come from one server-side producer
  // whose output also feeds the AI exposure fact, so the only way the human's
  // picture can diverge from the AI's is if this component invents geometry.
  // These tests are what stops that.

  const contact = (over) => Object.assign({
    uuid: 'hostile-1',
    x: 0,
    z: -100,
    arcs: [{ bearing_deg: 180, half_angle_deg: 45, range: 200 }],
  }, over || {});

  const baseState = (over) => Object.assign({
    range: 500, x: 0, z: 0, heading: 0,
    hostile_arcs: [contact()],
    hostile_arc_color: [1, 0.3, 0.3, 0.07],
  }, over || {});

  it('renders one wedge per arc handed to it, and no more', () => {
    const { el } = setup();
    const g = el.shadowRoot.getElementById('hostile-arcs');

    el.state = baseState();
    expect(g.children.length).toBe(1);

    el.state = baseState({
      hostile_arcs: [
        contact({ arcs: [
          { bearing_deg: 0, half_angle_deg: 30, range: 200 },
          { bearing_deg: 180, half_angle_deg: 30, range: 200 },
        ] }),
        contact({ uuid: 'hostile-2', x: 50 }),
      ],
    });
    expect(g.children.length).toBe(3);
  });

  it('renders no wedges when handed no arcs — the red-alert gate', () => {
    const { el } = setup();
    const g = el.shadowRoot.getElementById('hostile-arcs');

    el.state = baseState();
    expect(g.children.length).toBe(1);

    // What buildHelmConsoleState sends when the ship is not at red alert.
    el.state = baseState({ hostile_arcs: [] });
    expect(g.children.length).toBe(0);
  });

  it('anchors the wedge at the contact, not at the scope centre', () => {
    const { el } = setup();
    const g = el.shadowRoot.getElementById('hostile-arcs');
    // Contact 100 units dead ahead on a 500-unit scope => a fifth of the
    // radius above centre => (50, 40) in the 100x100 overlay.
    el.state = baseState();
    const d = g.children[0].getAttribute('d');
    expect(d.startsWith('M 50.0 40.0 ')).toBe(true);
  });

  it('pins the whole wedge — anchor, sweep and radius — to the sector given', () => {
    const { el } = setup();
    const g = el.shadowRoot.getElementById('hostile-arcs');
    // Contact 100 ahead on a 500 scope, arc bearing 180 (pointing back at us),
    // half-angle 45, reach 200. Anchor (50, 40), radius 20, wedge spanning
    // screen angles 45 -> 135 degrees. Every number here is a projection of a
    // number the server sent; none of it is derived from the hostile's yaw.
    el.state = baseState();
    expect(g.children[0].getAttribute('d'))
      .toBe('M 50.0 40.0 L 64.1 54.1 A 20.0 20.0 0 0 1 35.9 54.1 Z');
  });

  it('rotates the wedge by the ship heading, in the right direction', () => {
    const { el } = setup();
    const g = el.shadowRoot.getElementById('hostile-arcs');
    // Same contact, ship now heading 90. The contact falls abeam to port and
    // the world-180 arc becomes a screen-relative 90 — subtracting the
    // heading, not adding it.
    el.state = baseState({ heading: 90 });
    expect(g.children[0].getAttribute('d'))
      .toBe('M 40.0 50.0 L 54.1 35.9 A 20.0 20.0 0 0 1 54.1 64.1 Z');
  });

  it('projects world bearing through the ship heading and nothing else', () => {
    const { el } = setup();
    const g = el.shadowRoot.getElementById('hostile-arcs');
    // A contact directly astern of us with an arc pointing back at us
    // (bearing 180 world). With the ship heading 0 the wedge spans
    // 180 +/- 45 in screen terms; rotating the SHIP by 90 rotates the wedge by
    // -90 and nothing else about it changes.
    el.state = baseState();
    const straight = g.children[0].getAttribute('d');
    el.state = baseState({ heading: 90 });
    const rotated = g.children[0].getAttribute('d');
    expect(rotated).not.toBe(straight);
    // Same wedge, re-derived by hand: heading 90 puts the contact abeam to
    // port, so the anchor moves to (40, 50).
    expect(rotated.startsWith('M 40.0 50.0 ')).toBe(true);
  });

  it('sizes the wedge radius from the arc range and the scope range', () => {
    const { el } = setup();
    const g = el.shadowRoot.getElementById('hostile-arcs');
    // range 200 on a 500 scope => 0.4 * 50 = 20.0 overlay units.
    el.state = baseState();
    expect(g.children[0].getAttribute('d')).toContain('A 20.0 20.0 ');
    // Halve the arc's reach, halve the radius. Nothing else moves.
    el.state = baseState({ hostile_arcs: [contact({ arcs: [{ bearing_deg: 180, half_angle_deg: 45, range: 100 }] })] });
    expect(g.children[0].getAttribute('d')).toContain('A 10.0 10.0 ');
  });

  it('takes its fill colour and opacity from the payload, not from JS', () => {
    const { el } = setup();
    const g = el.shadowRoot.getElementById('hostile-arcs');

    el.state = baseState({ hostile_arc_color: [1, 0, 0, 0.05] });
    expect(g.children[0].getAttribute('fill')).toBe('rgb(255,0,0)');
    expect(g.children[0].getAttribute('fill-opacity')).toBe('0.05');

    el.state = baseState({ hostile_arc_color: [0, 0.5, 1, 0.2] });
    expect(g.children[0].getAttribute('fill')).toBe('rgb(0,128,255)');
    expect(g.children[0].getAttribute('fill-opacity')).toBe('0.2');
  });

  // Walk a path and report whether every `A` command actually draws: per the
  // SVG spec (implementation notes F.6.2) an elliptical arc whose endpoints are
  // identical is omitted entirely, so such a segment contributes no area and,
  // with `.hostile-arc { stroke: none }`, nothing visible.
  const everyArcSegmentDraws = (d) => {
    const t = d.trim().split(/\s+/);
    let cur = null;
    let ok = true;
    for (let i = 0; i < t.length; i++) {
      if (t[i] === 'M' || t[i] === 'L') {
        cur = [Number(t[i + 1]), Number(t[i + 2])];
        i += 2;
      } else if (t[i] === 'A') {
        const end = [Number(t[i + 6]), Number(t[i + 7])];
        if (cur && cur[0] === end[0] && cur[1] === end[1]) ok = false;
        cur = end;
        i += 7;
      }
    }
    return ok;
  };

  it('paints a full disc for a 360-degree bank, not a collapsed arc', () => {
    const { el } = setup();
    const g = el.shadowRoot.getElementById('hostile-arcs');
    // `assets/entities/alliance_destroyer.toml` authors a `fire_arc_deg = 360`
    // `omni` suppression phaser — half_angle_deg 180 — so this is a real
    // shipped bank, not a synthetic edge case. The AI's `arc_exposure`
    // reads a 180-degree half-angle as always covering; if the overlay drew
    // nothing here, human and AI would hold different information (AC4).
    el.state = baseState({
      hostile_arcs: [contact({ arcs: [{ bearing_deg: 0, half_angle_deg: 180, range: 200 }] })],
    });
    expect(g.children.length).toBe(1);

    const d = g.children[0].getAttribute('d');
    expect(everyArcSegmentDraws(d)).toBe(true);
    // Anchor (50, 40), radius 20: a closed disc reaching the full radius on
    // every side of the contact's blip, drawn as two half-circles.
    expect(d).toBe('M 50.0 20.0 A 20.0 20.0 0 1 1 50.0 60.0 A 20.0 20.0 0 1 1 50.0 20.0 Z');
  });

  it('paints a full disc for a bank wider than a full turn, not a notched one', () => {
    const { el } = setup();
    const g = el.shadowRoot.getElementById('hostile-arcs');
    // A half-angle past 180 wraps PAST its own start, so the two endpoints stop
    // coinciding and the endpoint test alone lets it through — drawing a disc
    // with a notch cut out of it, covering less than the circle. `arc_exposure`
    // reads any half-angle >= 180 as inescapable from every bearing, so the
    // notch is the same human/AI divergence in a third disguise. Nothing
    // authors past 360 today and `weapon_arc_sectors` does not clamp, so this
    // pins the guard rather than a reachable defect.
    el.state = baseState({
      hostile_arcs: [contact({ arcs: [{ bearing_deg: 0, half_angle_deg: 190, range: 200 }] })],
    });
    expect(g.children.length).toBe(1);

    const d = g.children[0].getAttribute('d');
    expect(everyArcSegmentDraws(d)).toBe(true);
    expect(d).toBe('M 50.0 20.0 A 20.0 20.0 0 1 1 50.0 60.0 A 20.0 20.0 0 1 1 50.0 20.0 Z');
  });

  it('draws every non-degenerate sector width as a segment with area', () => {
    const { el } = setup();
    const g = el.shadowRoot.getElementById('hostile-arcs');
    // The last three are the ones that matter. A sweep just UNDER a full turn
    // leaves a residual gap between the wedge's two endpoints that `toFixed(1)`
    // rounds away once the screen radius is small enough, and per SVG F.6.2 an
    // `A` whose endpoints coincide is omitted — so the wedge paints nothing
    // while `arc_exposure` still reads the bank as covering. Screen radius is
    // `(range / scope_range) * 50` and the scope here is 500, so range 20/50/100
    // put these at radius 2/5/10: exactly the band where each of 358/359/359.5
    // collapses. A short-ranged bank on a wide scope lands here routinely, so
    // the fix has to key off the EMITTED endpoints, not off `halfDeg * 2 >= 360`.
    const cases = [
      { half: 1, range: 200 },
      { half: 15, range: 200 },
      { half: 45, range: 200 },
      { half: 90, range: 200 },
      { half: 135, range: 200 },
      { half: 179, range: 200 },
      { half: 180, range: 200 },
      { half: 179, range: 20 },      // sweep 358   at screen radius 2
      { half: 179.5, range: 50 },    // sweep 359   at screen radius 5
      { half: 179.75, range: 100 },  // sweep 359.5 at screen radius 10
    ];
    for (const { half, range } of cases) {
      el.state = baseState({
        hostile_arcs: [contact({ arcs: [{ bearing_deg: 0, half_angle_deg: half, range }] })],
      });
      expect(g.children.length).toBe(1);
      expect(everyArcSegmentDraws(g.children[0].getAttribute('d'))).toBe(true);
    }
  });

  it('ignores degenerate sectors rather than drawing a zero wedge', () => {
    const { el } = setup();
    const g = el.shadowRoot.getElementById('hostile-arcs');
    el.state = baseState({
      hostile_arcs: [contact({ arcs: [
        { bearing_deg: 0, half_angle_deg: 0, range: 200 },
        { bearing_deg: 0, half_angle_deg: 30, range: 0 },
      ] })],
    });
    expect(g.children.length).toBe(0);
  });
});
