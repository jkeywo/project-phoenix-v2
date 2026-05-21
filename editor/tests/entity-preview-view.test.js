import { describe, it, expect, beforeEach } from 'vitest';
import { renderEntityPreviewView } from '../entity-preview-view.js';
import { computeEntityPreview } from '../entity-preview.js';

// ── Minimal DOM shim ────────────────────────────────────────────────────

class FakeClassList {
  constructor() { this._set = new Set(); }
  add(c) { this._set.add(c); }
  remove(c) { this._set.delete(c); }
  contains(c) { return this._set.has(c); }
}

class FakeElement {
  constructor(tag) {
    this.tagName = (tag || 'div').toUpperCase();
    this.children = [];
    this.parentElement = null;
    this.classList = new FakeClassList();
    this._innerHTML = '';
    this.textContent = '';
  }
  set className(v) {
    this.classList = new FakeClassList();
    for (const c of String(v || '').split(/\s+/).filter(Boolean)) this.classList.add(c);
    this._className = v;
  }
  get className() { return this._className || ''; }
  appendChild(c) {
    if (c.parentElement) {
      const i = c.parentElement.children.indexOf(c);
      if (i >= 0) c.parentElement.children.splice(i, 1);
    }
    c.parentElement = this;
    this.children.push(c);
    return c;
  }
  set innerHTML(v) { this._innerHTML = v; this.children = []; }
  get innerHTML() { return this._innerHTML; }
  _walk(pred, out = []) {
    if (pred(this)) out.push(this);
    for (const c of this.children) c._walk(pred, out);
    return out;
  }
}

class FakeDocument {
  createElement(tag) { return new FakeElement(tag); }
}

function installDom() {
  globalThis.document = new FakeDocument();
}

// ── Mock Konva (records every shape constructed) ────────────────────────

function makeMockKonva() {
  const created = [];
  function mk(name) {
    return class {
      constructor(opts) {
        this._kind = name;
        this._opts = opts;
        created.push({ kind: name, opts });
      }
    };
  }
  const Stage = class {
    constructor(opts) { this._opts = opts; this._layers = []; created.push({ kind: 'Stage', opts }); }
    add(l) { this._layers.push(l); }
  };
  const Layer = class {
    constructor() { this._children = []; created.push({ kind: 'Layer' }); }
    add(s) { this._children.push(s); }
    draw() {}
  };
  return {
    Konva: {
      Stage, Layer,
      Circle: mk('Circle'),
      Rect: mk('Rect'),
      Ring: mk('Ring'),
      Line: mk('Line'),
      Arc: mk('Arc'),
      RegularPolygon: mk('RegularPolygon'),
    },
    created,
  };
}

// ── Tests ───────────────────────────────────────────────────────────────

describe('renderEntityPreviewView', () => {
  let host;

  beforeEach(() => {
    installDom();
    host = new FakeElement('div');
  });

  it('renders placeholder when preview is placeholder', () => {
    const { Konva } = makeMockKonva();
    renderEntityPreviewView(host, { placeholder: true, activeFile: null }, { Konva });
    expect(host.children).toHaveLength(1);
    expect(host.children[0].classList.contains('placeholder')).toBe(true);
  });

  it('renders ship: capsule collider + triangle radar + forward arrow', () => {
    const preview = computeEntityPreview({
      tags: ['ship'],
      collider: { shape: 'Capsule', radius: 3.0, length: 6.0 },
      radar_appearance: { colour: [0.6, 0.8, 1.0], radius: 5.0 },
    });
    const { Konva, created } = makeMockKonva();
    renderEntityPreviewView(host, preview, { Konva });

    const kinds = created.map((c) => c.kind);
    // Capsule = 2 lines + 2 arcs
    expect(kinds.filter((k) => k === 'Line').length).toBe(2);
    expect(kinds.filter((k) => k === 'Arc').length).toBe(2);
    // Triangle radar + forward arrow = 2 RegularPolygons
    expect(kinds.filter((k) => k === 'RegularPolygon').length).toBe(2);
  });

  it('renders station with diamond radar', () => {
    const preview = computeEntityPreview({
      tags: ['station'],
      radar_appearance: { colour: [0.3, 0.8, 0.6], radius: 10.0 },
    });
    const { Konva, created } = makeMockKonva();
    renderEntityPreviewView(host, preview, { Konva });

    // Diamond is a RegularPolygon with sides=4
    const polys = created.filter((c) => c.kind === 'RegularPolygon');
    const diamond = polys.find((p) => p.opts.sides === 4);
    expect(diamond).toBeDefined();
  });

  it('renders region with sphere shape (dashed circle)', () => {
    const preview = computeEntityPreview({
      tags: ['region'],
      shape: { type: 'sphere', radius: 100.0 },
    });
    const { Konva, created } = makeMockKonva();
    renderEntityPreviewView(host, preview, { Konva });

    const circles = created.filter((c) => c.kind === 'Circle');
    // One dashed sphere circle.
    const dashed = circles.find((c) => Array.isArray(c.opts.dash));
    expect(dashed).toBeDefined();
  });

  it('renders region with torus shape as a ring', () => {
    const preview = computeEntityPreview({
      tags: ['region'],
      shape: { type: 'torus', inner_radius: 100, outer_radius: 250 },
    });
    const { Konva, created } = makeMockKonva();
    renderEntityPreviewView(host, preview, { Konva });

    const rings = created.filter((c) => c.kind === 'Ring');
    expect(rings.length).toBeGreaterThan(0);
    const r = rings[0];
    expect(r.opts.outerRadius).toBeGreaterThan(r.opts.innerRadius);
  });

  it('renders asteroid_field donut', () => {
    const preview = computeEntityPreview({
      tags: ['asteroid_field'],
      asteroid_field: { inner_radius: 100, outer_radius: 200, density: 0.005 },
    });
    const { Konva, created } = makeMockKonva();
    renderEntityPreviewView(host, preview, { Konva });

    const rings = created.filter((c) => c.kind === 'Ring');
    expect(rings.length).toBeGreaterThan(0);
  });

  it('overlay shows resolved faction name (not raw UUID)', () => {
    const factionMap = new Map([['ff-1', 'Federation']]);
    const preview = computeEntityPreview(
      { tags: ['ship'], faction: 'ff-1' },
      factionMap,
    );
    const { Konva } = makeMockKonva();
    renderEntityPreviewView(host, preview, { Konva });

    const overlay = host.children.find((c) => c.classList.contains('entity-preview-overlay'));
    expect(overlay).toBeDefined();
    const text = overlay.children.map((r) => r.textContent).join('\n');
    expect(text).toContain('Federation');
    expect(text).not.toContain('ff-1');
  });

  it('emits an error placeholder when Konva is unavailable', () => {
    renderEntityPreviewView(host, { textOverlay: { tags: [] }, showForwardArrow: false }, { Konva: null });
    expect(host.children).toHaveLength(1);
    expect(host.children[0].classList.contains('entity-preview-error')).toBe(true);
  });
});
