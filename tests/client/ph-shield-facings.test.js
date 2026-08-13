// @vitest-environment jsdom
import { t } from '../../gui/strings.js';
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import '../../gui/components/ph-shield-facings.js';

function setup(opts) {
  if (opts && opts.sendAction) {
    window.sendAction = opts.sendAction;
  }
  document.body.innerHTML = '<ph-shield-facings id="test-el"></ph-shield-facings>';
  const el = document.getElementById('test-el');
  return { el };
}

function queryText(host, sel) {
  const el = host.shadowRoot.querySelector(sel);
  return el ? el.textContent.trim() : null;
}

/** Distance of a path's initial `M x y` point from the donut centre (100,100). */
function hitPathOuterRadius(d) {
  const m = /^M\s*([\d.+-]+)[ ,]+([\d.+-]+)/.exec(d || '');
  if (!m) return null;
  return Math.hypot(parseFloat(m[1]) - 100, parseFloat(m[2]) - 100);
}

/** Angle (radians) of a point relative to the donut centre (100,100). */
function angleOf(x, y) {
  return Math.atan2(y - 100, x - 100);
}

/**
 * Parses the outer-arc start/end points from a wedge's `d` attribute
 * (`M x0 y0 A rx ry rot largeArc sweep x1 y1 L ...`). Works for both the
 * visual .arc-path and the .hit-path — both are built from the same
 * `M`/`A`/`L`/`A`/`Z` template, so the M point and the first A command's
 * endpoint are always tokens [1],[2] and [9],[10].
 */
function outerArcPoints(d) {
  const tokens = (d || '').trim().split(/[\s,]+/);
  return {
    x0: parseFloat(tokens[1]), y0: parseFloat(tokens[2]),
    x1: parseFloat(tokens[9]), y1: parseFloat(tokens[10]),
  };
}

describe('PhShieldFacings', () => {
  beforeEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
  });

  afterEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
    vi.useRealTimers();
  });

  it('is defined and registered as a custom element', () => {
    expect(customElements.get('ph-shield-facings')).toBeDefined();
  });

  it('creates a shadow root', () => {
    const { el } = setup();
    expect(el.shadowRoot).toBeDefined();
  });

  it('renders empty state with no facing data', () => {
    const { el } = setup();
    el.state = {};
    expect(el.shadowRoot.textContent).toContain(t('component.shield_facings.empty'));
  });

  it('renders empty state when facings is null', () => {
    const { el } = setup();
    el.state = { facings: null };
    expect(el.shadowRoot.textContent).toContain(t('component.shield_facings.empty'));
  });

  it('renders facings with SVG arcs', () => {
    const { el } = setup();
    el.state = {
      facings: [
        { arc_id: 'fore', label: 'Fore', hp: 100, max_hp: 100, online: true },
        { arc_id: 'port', label: 'Port', hp: 50, max_hp: 100, online: true },
        { arc_id: 'aft', label: 'Aft', hp: 0, max_hp: 100, online: false },
      ],
      focused_facing: 'port',
      system_id: 'shields-system',
      auto: false,
    };
    const svg = el.shadowRoot.querySelector('svg');
    expect(svg).toBeDefined();
    expect(el.shadowRoot.textContent).toContain('FORE');
    expect(el.shadowRoot.textContent).toContain('PORT');
    expect(el.shadowRoot.textContent).toContain('AFT');
  });

  it('shows OFF label for offline facing', () => {
    const { el } = setup();
    el.state = {
      facings: [
        { arc_id: 'aft', label: 'Aft', hp: 0, max_hp: 100, online: false },
      ],
    };
    expect(el.shadowRoot.textContent).toContain('OFF');
  });

  it('shows HP percentage for online facing', () => {
    const { el } = setup();
    el.state = {
      facings: [
        { arc_id: 'fore', label: 'Fore', hp: 100, max_hp: 100, online: true },
      ],
    };
    expect(el.shadowRoot.textContent).toContain('100%');
  });

  it('shows AUTO badge and disables interaction when auto=true', () => {
    const { el } = setup();
    el.state = {
      facings: [{ arc_id: 'fore', label: 'Fore', hp: 100, max_hp: 100, online: true }],
      auto: true,
    };
    const badge = el.shadowRoot.getElementById('auto-badge');
    expect(badge.style.display).not.toBe('none');
  });

  it('hides AUTO badge when auto=false', () => {
    const { el } = setup();
    el.state = {
      facings: [{ arc_id: 'fore', label: 'Fore', hp: 100, max_hp: 100, online: true }],
      auto: false,
    };
    const badge = el.shadowRoot.getElementById('auto-badge');
    expect(badge.style.display).toBe('none');
  });

  // The hit-path (not .arc-path) now carries the sole click listener — it is
  // the enlarged, topmost touch target (#1009) — so interaction tests dispatch
  // on it. The wire payload itself is unchanged.
  it('clicking an unfocused facing arc dispatches set_shield_focus with focused: true', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      facings: [
        { arc_id: 'fore', label: 'Fore', hp: 100, max_hp: 100, online: true },
      ],
      focused_facing: null,
      auto: false,
    };
    const hit = el.shadowRoot.querySelector('.hit-path');
    expect(hit).toBeDefined();
    hit.dispatchEvent(new MouseEvent('click'));
    expect(sendAction).toHaveBeenCalledWith('set_shield_focus', { arc_id: 'fore', focused: true });
  });

  it('clicking the already-focused facing arc dispatches set_shield_focus with focused: false', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      facings: [
        { arc_id: 'fore', label: 'Fore', hp: 100, max_hp: 100, online: true },
      ],
      focused_facing: 'fore',
      auto: false,
    };
    const hit = el.shadowRoot.querySelector('.hit-path');
    hit.dispatchEvent(new MouseEvent('click'));
    expect(sendAction).toHaveBeenCalledWith('set_shield_focus', { arc_id: 'fore', focused: false });
  });

  it('does not dispatch action when auto=true', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      facings: [
        { arc_id: 'fore', label: 'Fore', hp: 100, max_hp: 100, online: true },
      ],
      auto: true,
    };
    const hit = el.shadowRoot.querySelector('.hit-path');
    hit.dispatchEvent(new MouseEvent('click'));
    expect(sendAction).not.toHaveBeenCalled();
  });

  // ── #1009: press feedback + enlarged touch target ───────────────────────

  it('flashes the arc and shows the AUTO hint on an auto-mode press instead of doing nothing', () => {
    vi.useFakeTimers();
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      facings: [{ arc_id: 'fore', label: 'Fore', hp: 100, max_hp: 100, online: true }],
      auto: true,
    };
    const outline = el.shadowRoot.querySelector('.arc-path');
    const hint = el.shadowRoot.getElementById('auto-hint');
    expect(outline.classList.contains('press-flash')).toBe(false);
    expect(hint.classList.contains('show')).toBe(false);

    const hit = el.shadowRoot.querySelector('.hit-path');
    hit.dispatchEvent(new MouseEvent('click'));

    expect(outline.classList.contains('press-flash')).toBe(true);
    expect(hint.classList.contains('show')).toBe(true);
    expect(hint.textContent).toBe(t('component.shield_facings.auto_hint'));
    expect(sendAction).not.toHaveBeenCalled();
    vi.useRealTimers();
  });

  it('re-triggers the flash animation on a second auto-mode press (remove+reflow+add)', () => {
    const { el } = setup();
    el.state = {
      facings: [{ arc_id: 'fore', label: 'Fore', hp: 100, max_hp: 100, online: true }],
      auto: true,
    };
    const outline = el.shadowRoot.querySelector('.arc-path');
    const hit = el.shadowRoot.querySelector('.hit-path');

    hit.dispatchEvent(new MouseEvent('click'));
    expect(outline.classList.contains('press-flash')).toBe(true);

    const removeSpy = vi.spyOn(outline.classList, 'remove');
    const addSpy = vi.spyOn(outline.classList, 'add');
    hit.dispatchEvent(new MouseEvent('click'));

    expect(removeSpy).toHaveBeenCalledWith('press-flash');
    expect(addSpy).toHaveBeenCalledWith('press-flash');
    expect(outline.classList.contains('press-flash')).toBe(true);
  });

  it('hides the AUTO hint again after its timeout', () => {
    vi.useFakeTimers();
    const { el } = setup();
    el.state = {
      facings: [{ arc_id: 'fore', label: 'Fore', hp: 100, max_hp: 100, online: true }],
      auto: true,
    };
    const hit = el.shadowRoot.querySelector('.hit-path');
    const hint = el.shadowRoot.getElementById('auto-hint');

    hit.dispatchEvent(new MouseEvent('click'));
    expect(hint.classList.contains('show')).toBe(true);

    vi.advanceTimersByTime(2500);
    expect(hint.classList.contains('show')).toBe(false);
  });

  it('also flashes the arc on a normal (non-auto) press, and still sends the unchanged payload', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      facings: [{ arc_id: 'fore', label: 'Fore', hp: 100, max_hp: 100, online: true }],
      focused_facing: null,
      auto: false,
    };
    const outline = el.shadowRoot.querySelector('.arc-path');
    const hint = el.shadowRoot.getElementById('auto-hint');
    const hit = el.shadowRoot.querySelector('.hit-path');

    hit.dispatchEvent(new MouseEvent('click'));

    expect(outline.classList.contains('press-flash')).toBe(true);
    // The AUTO hint is reserved for the swallowed auto-mode press.
    expect(hint.classList.contains('show')).toBe(false);
    expect(sendAction).toHaveBeenCalledWith('set_shield_focus', { arc_id: 'fore', focused: true });
    expect(sendAction).toHaveBeenCalledTimes(1);
  });

  it('enlarges the arc touch target radially, but not angularly (#1009)', () => {
    const { el } = setup();
    el.state = {
      facings: [{ arc_id: 'fore', label: 'Fore', hp: 100, max_hp: 100, online: true }],
    };
    const outline = el.shadowRoot.querySelector('.arc-path');
    const hit = el.shadowRoot.querySelector('.hit-path');
    expect(hit).toBeTruthy();
    expect(hit).not.toBe(outline);

    const visualR = hitPathOuterRadius(outline.getAttribute('d'));
    const hitR = hitPathOuterRadius(hit.getAttribute('d'));
    expect(visualR).toBeCloseTo(70, 0);
    expect(hitR).toBeGreaterThan(visualR);

    // Radial padding only: the hit path's start/end points sit on the same
    // rays from centre as the visual wedge's, just further out. Widening
    // the angle here would poach a neighbouring facing's own slice — see
    // the adjacency tests below, which is why there's no angular pad at all.
    const outlinePts = outerArcPoints(outline.getAttribute('d'));
    const hitPts = outerArcPoints(hit.getAttribute('d'));
    expect(angleOf(hitPts.x0, hitPts.y0)).toBeCloseTo(angleOf(outlinePts.x0, outlinePts.y0), 6);
    expect(angleOf(hitPts.x1, hitPts.y1)).toBeCloseTo(angleOf(outlinePts.x1, outlinePts.y1), 6);
  });

  // ── #1009 follow-up: contiguous facings must not fight over shared edges ──
  //
  // jsdom has no layout engine, so these tests can't do real coordinate
  // hit-testing (dispatching a click "at" an x/y and seeing which element
  // the browser would have picked). Instead the misfire is pinned two
  // separate ways: numerically, by proving the enlarged hit-paths' angular
  // spans don't overlap at the shared boundary (so no pixel could ever be
  // claimed by two hit-paths at once); and at the wiring level, by proving
  // each hit-path's own click listener reports its own facing's arc_id.

  it("adjacent facings' hit-paths meet at an exact shared angle with no overlap", () => {
    const { el } = setup();
    el.state = {
      facings: [
        { arc_id: 'fore', label: 'Fore', hp: 100, max_hp: 100, online: true },
        { arc_id: 'stbd', label: 'Stbd', hp: 100, max_hp: 100, online: true },
        { arc_id: 'aft', label: 'Aft', hp: 100, max_hp: 100, online: true },
      ],
    };
    const hits = Array.from(el.shadowRoot.querySelectorAll('.hit-path'));
    expect(hits).toHaveLength(3);
    const spans = hits.map(h => {
      const { x0, y0, x1, y1 } = outerArcPoints(h.getAttribute('d'));
      return { id: h.getAttribute('data-facing-id'), a0: angleOf(x0, y0), a1: angleOf(x1, y1) };
    });

    // Each facing's end angle is exactly the next facing's start angle:
    // contiguous and touching, but never overlapping into the neighbour.
    const normalize = (a) => ((a % (2 * Math.PI)) + 2 * Math.PI) % (2 * Math.PI);
    for (let i = 0; i < spans.length; i++) {
      const next = spans[(i + 1) % spans.length];
      expect(normalize(spans[i].a1)).toBeCloseTo(normalize(next.a0), 6);
    }
  });

  it("a click on one facing's hit-path always fires that facing's own arc_id, never a neighbour's", () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      facings: [
        { arc_id: 'fore', label: 'Fore', hp: 100, max_hp: 100, online: true },
        { arc_id: 'stbd', label: 'Stbd', hp: 100, max_hp: 100, online: true },
        { arc_id: 'aft', label: 'Aft', hp: 100, max_hp: 100, online: true },
      ],
      focused_facing: null,
      auto: false,
    };
    const hits = Array.from(el.shadowRoot.querySelectorAll('.hit-path'));
    expect(hits.map(h => h.getAttribute('data-facing-id'))).toEqual(['fore', 'stbd', 'aft']);

    hits[0].dispatchEvent(new MouseEvent('click'));
    expect(sendAction).toHaveBeenLastCalledWith('set_shield_focus', { arc_id: 'fore', focused: true });

    hits[1].dispatchEvent(new MouseEvent('click'));
    expect(sendAction).toHaveBeenLastCalledWith('set_shield_focus', { arc_id: 'stbd', focused: true });

    hits[2].dispatchEvent(new MouseEvent('click'));
    expect(sendAction).toHaveBeenLastCalledWith('set_shield_focus', { arc_id: 'aft', focused: true });

    expect(sendAction).toHaveBeenCalledTimes(3);
  });

  it('updates when state changes', () => {
    const { el } = setup();
    el.state = {
      facings: [{ arc_id: 'fore', label: 'Fore', hp: 100, max_hp: 100, online: true }],
      focused_facing: null,
    };
    expect(el.shadowRoot.textContent).toContain('FORE');
    el.state = {
      facings: [
        { arc_id: 'fore', label: 'Fore', hp: 100, max_hp: 100, online: true },
        { arc_id: 'port', label: 'Port', hp: 50, max_hp: 100, online: true },
      ],
      focused_facing: 'port',
    };
    expect(el.shadowRoot.textContent).toContain('PORT');
  });
});
